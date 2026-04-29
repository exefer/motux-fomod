use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::condition::{CompositeDependency, EvalContext, Evaluate};
use crate::config::{
    FileList, Group, GroupType, InstallStep, ModuleConfig, Plugin, PluginType, SortOrder,
};

/// A planned file operation: copy source to destination with priority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileOperation {
    pub source: String,
    pub destination: String,
    pub is_folder: bool,
    pub priority: i32,
}

/// The resolved installation plan after all user selections.
#[derive(Debug, Clone)]
pub struct InstallPlan {
    /// File operations sorted by priority (lower first, higher overwrites).
    pub operations: Vec<FileOperation>,
}

impl InstallPlan {
    /// Execute this plan by copying files from `source` to `destination`.
    ///
    /// `source` is the root of the unpacked mod archive. `destination` is the
    /// game's data directory (or staging folder). Operations are applied in
    /// priority order - higher-priority files overwrite lower.
    ///
    /// Intermediate directories are created as needed. An empty operation
    /// destination means the file keeps its source-relative path.
    pub fn execute(&self, source: &Path, destination: &Path) -> io::Result<()> {
        for op in &self.operations {
            let src = source.join(&op.source);
            let dst_rel = if op.destination.is_empty() {
                &op.source
            } else {
                &op.destination
            };
            let dst = destination.join(dst_rel);

            if op.is_folder {
                copy_dir_recursive(&src, &dst)?;
            } else {
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(&src, &dst)?;
            }
        }
        Ok(())
    }
}

/// Recursively copy a directory tree from `src` to `dst`.
fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let entry_dst = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &entry_dst)?;
        } else {
            fs::copy(entry.path(), &entry_dst)?;
        }
    }
    Ok(())
}

/// Apply `SortOrder` to items by name.
fn apply_sort_order<T>(items: &mut [T], order: Option<SortOrder>, name_fn: impl Fn(&T) -> &str) {
    match order {
        Some(SortOrder::Ascending) => items.sort_by(|a, b| name_fn(a).cmp(name_fn(b))),
        Some(SortOrder::Descending) => items.sort_by(|a, b| name_fn(b).cmp(name_fn(a))),
        Some(SortOrder::Explicit) | None => {} // document order
    }
}

/// Sort all steps, groups, and plugins in-place according to their `@order` attributes.
fn apply_sort_orders(config: &mut ModuleConfig) {
    if let Some(install_steps) = &mut config.install_steps {
        apply_sort_order(&mut install_steps.steps, install_steps.order, |s| &s.name);

        for step in &mut install_steps.steps {
            if let Some(groups) = &mut step.optional_file_groups {
                apply_sort_order(&mut groups.groups, groups.order, |g| &g.name);

                for group in &mut groups.groups {
                    apply_sort_order(&mut group.plugins.plugins, group.plugins.order, |p| &p.name);
                }
            }
        }
    }
}

/// Drives the FOMOD installation process.
///
/// Tracks user selections and condition flags, then resolves the final
/// set of file operations.
pub struct Installer {
    config: ModuleConfig,
    ctx: EvalContext,
    /// Maps (step_index, group_index) -> set of selected plugin indices.
    selections: HashMap<(usize, usize), Vec<usize>>,
    /// Undo history: snapshots of (selections, flags).
    history: Vec<SelectionSnapshot>,
}

impl Installer {
    pub fn new(config: ModuleConfig) -> Self {
        Self::with_context(config, EvalContext::new())
    }

    /// Create an installer with pre-populated game environment context.
    pub fn with_context(mut config: ModuleConfig, ctx: EvalContext) -> Self {
        apply_sort_orders(&mut config);
        Self {
            config,
            ctx,
            selections: HashMap::new(),
            history: Vec::new(),
        }
    }

    pub fn context(&self) -> &EvalContext {
        &self.ctx
    }

    pub fn context_mut(&mut self) -> &mut EvalContext {
        &mut self.ctx
    }

    pub fn config(&self) -> &ModuleConfig {
        &self.config
    }

    /// Check module-level dependencies. Returns `true` if satisfied.
    pub fn check_dependencies(&self) -> bool {
        self.config
            .module_dependencies
            .as_ref()
            .is_none_or(|d| d.evaluate(&self.ctx))
    }

    /// Get visible install steps (steps whose visibility conditions are met).
    pub fn visible_steps(&self) -> Vec<(usize, &InstallStep)> {
        let steps = match self.config.install_steps {
            Some(ref s) => &s.steps,
            None => return Vec::new(),
        };

        steps
            .iter()
            .enumerate()
            .filter(|(_, step)| step.visible.as_ref().is_none_or(|v| v.evaluate(&self.ctx)))
            .collect()
    }

    /// Record user selections for a group within a step.
    ///
    /// `plugin_indices` are indices into the group's plugin list.
    pub fn select(&mut self, step_index: usize, group_index: usize, plugin_indices: Vec<usize>) {
        self.selections
            .insert((step_index, group_index), plugin_indices);

        // Collect flag updates from group plugins, then apply them.
        let mut flags_to_clear: Vec<String> = Vec::new();
        let mut flags_to_set: Vec<(String, String)> = Vec::new();

        if let Some(group) = self.get_group(step_index, group_index) {
            // Gather all flag names from every plugin in this group (to clear)
            for plugin in &group.plugins.plugins {
                if let Some(flags) = &plugin.condition_flags {
                    for flag in &flags.flags {
                        flags_to_clear.push(flag.name.clone());
                    }
                }
            }
            // Gather flags from selected plugins (to set)
            for &idx in &self.selections[&(step_index, group_index)] {
                if let Some(plugin) = group.plugins.plugins.get(idx)
                    && let Some(flags) = &plugin.condition_flags
                {
                    for flag in &flags.flags {
                        flags_to_set.push((flag.name.clone(), flag.value.clone()));
                    }
                }
            }
        }

        for name in flags_to_clear {
            self.ctx.flags.remove(&name);
        }

        for (name, value) in flags_to_set {
            self.ctx.set_flag(name, value);
        }
    }

    /// Get the default selections for a group based on context-aware plugin types.
    ///
    /// Evaluates `dependencyType` patterns against the provided context to
    /// determine actual plugin types at runtime.
    pub fn default_selections_in_context(group: &Group, ctx: &EvalContext) -> Vec<usize> {
        compute_defaults(group, |p| p.plugin_type_in_context(ctx))
    }

    /// Get the default selections for a group based on static plugin types.
    ///
    /// For `dependencyType` descriptors, uses the default type without
    /// evaluating conditions. Suitable for template generation.
    pub fn default_selections(group: &Group) -> Vec<usize> {
        compute_defaults(group, |p| p.plugin_type())
    }

    /// Validate selections against group type constraints.
    pub fn validate_selection(group: &Group, selected: &[usize]) -> Result<(), SelectionError> {
        let count = selected.len();
        let max = group.plugins.plugins.len();

        if selected.iter().any(|&i| i >= max) {
            return Err(SelectionError::OutOfBounds);
        }

        match group.group_type {
            GroupType::SelectExactlyOne if count != 1 => Err(SelectionError::InvalidCount {
                expected: "exactly 1",
                got: count,
            }),
            GroupType::SelectAtMostOne if count > 1 => Err(SelectionError::InvalidCount {
                expected: "at most 1",
                got: count,
            }),
            GroupType::SelectAtLeastOne if count < 1 => Err(SelectionError::InvalidCount {
                expected: "at least 1",
                got: count,
            }),
            GroupType::SelectAll if count != max => Err(SelectionError::InvalidCount {
                expected: "all",
                got: count,
            }),
            _ => Ok(()),
        }
    }

    /// Resolve the final installation plan from all selections.
    pub fn resolve(&self) -> InstallPlan {
        InstallPlan {
            operations: self.collect_operations(true),
        }
    }

    /// Get the name of a step by index.
    pub fn step_name(&self, step: usize) -> Option<&str> {
        self.config
            .install_steps
            .as_ref()?
            .steps
            .get(step)
            .map(|s| s.name.as_str())
    }

    /// Get the name of a group within a step.
    pub fn group_name(&self, step: usize, group: usize) -> Option<&str> {
        self.get_group(step, group).map(|g| g.name.as_str())
    }

    /// Get a plugin's description text.
    pub fn plugin_description(&self, step: usize, group: usize, plugin: usize) -> Option<&str> {
        let p = self.get_plugin(step, group, plugin)?;
        p.description.as_deref()
    }

    /// Get a plugin's image path (relative to the mod archive root).
    pub fn plugin_image_path(&self, step: usize, group: usize, plugin: usize) -> Option<&str> {
        self.get_plugin(step, group, plugin)
            .and_then(|p| p.image.as_ref())
            .map(|img| img.path.as_str())
    }

    /// Get the module header image path.
    pub fn module_image_path(&self) -> Option<&str> {
        self.config
            .module_image
            .as_ref()
            .filter(|img| img.show_image)
            .map(|img| img.path.as_str())
    }

    /// Get the resolved plugin type in the current context.
    pub fn plugin_type_at(&self, step: usize, group: usize, plugin: usize) -> Option<PluginType> {
        self.get_plugin(step, group, plugin)
            .map(|p| p.plugin_type_in_context(&self.ctx))
    }

    /// Get the group type for a specific group.
    pub fn group_type_at(&self, step: usize, group: usize) -> Option<GroupType> {
        self.get_group(step, group).map(|g| g.group_type)
    }

    /// Resolve an image path from the FOMOD XML against the mod archive root.
    ///
    /// Performs case-insensitive file lookup since FOMOD is Windows-centric
    /// and paths may not match the actual filesystem case on Linux.
    pub fn resolve_image(&self, base_path: &Path, image_path: &str) -> Option<PathBuf> {
        resolve_path_case_insensitive(base_path, image_path)
    }

    /// Preview the file operations a specific plugin would contribute.
    pub fn preview_plugin(&self, step: usize, group: usize, plugin: usize) -> Vec<FileOperation> {
        self.get_plugin(step, group, plugin)
            .and_then(|p| p.files.as_ref())
            .map(files_to_ops)
            .unwrap_or_default()
    }

    /// Preview the install plan based on current selections (without conditional file installs).
    ///
    /// This gives a "what you've chosen so far" view, useful for showing
    /// running totals during the wizard.
    pub fn preview_current(&self) -> InstallPlan {
        InstallPlan {
            operations: self.collect_operations(false),
        }
    }

    /// Get overall completion status of the wizard.
    pub fn completion_status(&self) -> CompletionStatus {
        let steps = match self.config.install_steps {
            Some(ref s) => &s.steps,
            None => {
                return CompletionStatus {
                    total_steps: 0,
                    visible_steps: 0,
                    total_groups: 0,
                    satisfied_groups: 0,
                };
            }
        };

        let visible = self.visible_steps();
        let mut total_groups = 0;
        let mut satisfied_groups = 0;

        for &(step_idx, step) in &visible {
            if let Some(groups) = &step.optional_file_groups {
                for (group_idx, group) in groups.groups.iter().enumerate() {
                    total_groups += 1;
                    let sel = self
                        .selections
                        .get(&(step_idx, group_idx))
                        .cloned()
                        .unwrap_or_default();
                    if Self::validate_selection(group, &sel).is_ok() {
                        satisfied_groups += 1;
                    }
                }
            }
        }

        CompletionStatus {
            total_steps: steps.len(),
            visible_steps: visible.len(),
            total_groups,
            satisfied_groups,
        }
    }

    /// Check if all required groups have valid selections and installation can proceed.
    pub fn is_ready_to_install(&self) -> bool {
        let status = self.completion_status();
        status.total_groups > 0 && status.satisfied_groups == status.total_groups
    }

    /// Return `(step_idx, group_idx)` pairs for groups that still need user input.
    pub fn missing_selections(&self) -> Vec<(usize, usize)> {
        let mut missing = Vec::new();

        for &(step_idx, step) in &self.visible_steps() {
            if let Some(groups) = &step.optional_file_groups {
                for (group_idx, group) in groups.groups.iter().enumerate() {
                    let sel = self
                        .selections
                        .get(&(step_idx, group_idx))
                        .cloned()
                        .unwrap_or_default();
                    if Self::validate_selection(group, &sel).is_err() {
                        missing.push((step_idx, group_idx));
                    }
                }
            }
        }

        missing
    }

    /// Validate all groups in a step and return detailed hints for the UI.
    pub fn validate_step(&self, step_index: usize) -> Vec<ValidationHint> {
        let Some(step) = self
            .config
            .install_steps
            .as_ref()
            .and_then(|s| s.steps.get(step_index))
        else {
            return Vec::new();
        };

        let mut hints = Vec::new();
        if let Some(groups) = &step.optional_file_groups {
            for (group_idx, group) in groups.groups.iter().enumerate() {
                let sel = self
                    .selections
                    .get(&(step_index, group_idx))
                    .cloned()
                    .unwrap_or_default();

                let count = sel.len();
                let max = group.plugins.plugins.len();

                match group.group_type {
                    GroupType::SelectExactlyOne if count != 1 => {
                        hints.push(ValidationHint::NeedExactly {
                            group: group.name.clone(),
                            required: 1,
                            current: count,
                        });
                    }
                    GroupType::SelectAtMostOne if count > 1 => {
                        hints.push(ValidationHint::ExceedsMax {
                            group: group.name.clone(),
                            max: 1,
                            current: count,
                        });
                    }
                    GroupType::SelectAtLeastOne if count < 1 => {
                        hints.push(ValidationHint::NeedAtLeast {
                            group: group.name.clone(),
                            required: 1,
                            current: count,
                        });
                    }
                    GroupType::SelectAll if count != max => {
                        hints.push(ValidationHint::NeedExactly {
                            group: group.name.clone(),
                            required: max,
                            current: count,
                        });
                    }
                    _ => {}
                }

                // Flag NotUsable plugins that are selected
                for &idx in &sel {
                    if let Some(plugin) = group.plugins.plugins.get(idx)
                        && plugin.plugin_type_in_context(&self.ctx) == PluginType::NotUsable
                    {
                        hints.push(ValidationHint::NotUsableSelected {
                            group: group.name.clone(),
                            plugin: plugin.name.clone(),
                        });
                    }
                }
            }
        }

        hints
    }

    /// Detect file conflicts: plugins that install to the same destination path.
    pub fn detect_conflicts(&self) -> Vec<FileConflict> {
        let mut dest_map: HashMap<String, Vec<FileConflictSource>> = HashMap::new();

        // Check required files
        if let Some(files) = &self.config.required_install_files {
            for item in &files.items {
                let r = item.file_ref();
                let dest = normalize_dest(&r.source, &r.destination);
                dest_map
                    .entry(dest)
                    .or_default()
                    .push(FileConflictSource::Required {
                        source: r.source.clone(),
                    });
            }
        }

        // Check plugin files
        if let Some(install_steps) = &self.config.install_steps {
            for (step_idx, step) in install_steps.steps.iter().enumerate() {
                if let Some(groups) = &step.optional_file_groups {
                    for (group_idx, group) in groups.groups.iter().enumerate() {
                        for (plugin_idx, plugin) in group.plugins.plugins.iter().enumerate() {
                            if let Some(files) = &plugin.files {
                                for item in &files.items {
                                    let r = item.file_ref();
                                    let dest = normalize_dest(&r.source, &r.destination);
                                    dest_map.entry(dest).or_default().push(
                                        FileConflictSource::Plugin {
                                            step: step_idx,
                                            group: group_idx,
                                            plugin: plugin_idx,
                                            plugin_name: plugin.name.clone(),
                                            source: r.source.clone(),
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        dest_map
            .into_iter()
            .filter(|(_, sources)| sources.len() > 1)
            .map(|(destination, sources)| FileConflict {
                destination,
                sources,
            })
            .collect()
    }

    /// Build a map of which plugins set flags that affect other steps' visibility.
    ///
    /// Returns a list of impacts: "selecting plugin X may show/hide step Y".
    pub fn flag_impact_map(&self) -> Vec<FlagImpact> {
        let steps = match self.config.install_steps {
            Some(ref s) => &s.steps,
            None => return Vec::new(),
        };

        // Collect all flags set by plugins
        let mut flag_setters: Vec<(usize, usize, usize, String, String)> = Vec::new(); // step, group, plugin, flag_name, flag_value

        for (step_idx, step) in steps.iter().enumerate() {
            if let Some(groups) = &step.optional_file_groups {
                for (group_idx, group) in groups.groups.iter().enumerate() {
                    for (plugin_idx, plugin) in group.plugins.plugins.iter().enumerate() {
                        if let Some(flags) = &plugin.condition_flags {
                            for flag in &flags.flags {
                                flag_setters.push((
                                    step_idx,
                                    group_idx,
                                    plugin_idx,
                                    flag.name.clone(),
                                    flag.value.clone(),
                                ));
                            }
                        }
                    }
                }
            }
        }

        // Check which steps have visibility conditions that reference these flags
        let mut impacts = Vec::new();
        for (step_idx, step) in steps.iter().enumerate() {
            if let Some(vis) = &step.visible {
                let referenced_flags = collect_flag_names(vis);
                for (src_step, src_group, src_plugin, flag_name, _) in &flag_setters {
                    if referenced_flags.contains(flag_name) {
                        impacts.push(FlagImpact {
                            source_step: *src_step,
                            source_group: *src_group,
                            source_plugin: *src_plugin,
                            flag_name: flag_name.clone(),
                            affected_step: step_idx,
                            affected_step_name: step.name.clone(),
                        });
                    }
                }
            }
        }

        impacts
    }

    /// Save a snapshot of current selections and context for later rollback.
    pub fn checkpoint(&mut self) {
        self.history.push(SelectionSnapshot {
            selections: self.selections.clone(),
            flags: self.ctx.flags.clone(),
        });
    }

    /// Rollback to the most recent checkpoint. Returns `false` if no history.
    pub fn rollback(&mut self) -> bool {
        if let Some(snapshot) = self.history.pop() {
            self.selections = snapshot.selections;
            self.ctx.flags = snapshot.flags;
            true
        } else {
            false
        }
    }

    /// Number of available undo checkpoints.
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Get current selections (read-only).
    pub fn selections(&self) -> &HashMap<(usize, usize), Vec<usize>> {
        &self.selections
    }

    fn collect_operations(&self, include_conditional: bool) -> Vec<FileOperation> {
        let mut ops: Vec<FileOperation> = Vec::new();

        // 1. Required install files (always installed)
        if let Some(files) = &self.config.required_install_files {
            ops.extend(files_to_ops(files));
        }

        // 2. Files from selected plugins
        for (&(step_idx, group_idx), selected) in &self.selections {
            if let Some(group) = self.get_group(step_idx, group_idx) {
                for &plugin_idx in selected {
                    if let Some(plugin) = group.plugins.plugins.get(plugin_idx)
                        && let Some(files) = &plugin.files
                    {
                        ops.extend(files_to_ops(files));
                    }
                }
            }
        }

        // 3. Conditional file installs (patterns evaluated against final flags)
        if include_conditional && let Some(cfi) = &self.config.conditional_file_installs {
            for pattern in &cfi.patterns.patterns {
                if pattern.dependencies.evaluate(&self.ctx) {
                    ops.extend(files_to_ops(&pattern.files));
                }
            }
        }

        ops.sort_by_key(|op| op.priority);

        ops
    }

    fn get_group(&self, step_index: usize, group_index: usize) -> Option<&Group> {
        self.config
            .install_steps
            .as_ref()?
            .steps
            .get(step_index)?
            .optional_file_groups
            .as_ref()?
            .groups
            .get(group_index)
    }

    fn get_plugin(&self, step: usize, group: usize, plugin: usize) -> Option<&Plugin> {
        let g = self.get_group(step, group)?;
        g.plugins.plugins.get(plugin)
    }
}

/// Snapshot of installer state for undo support.
#[derive(Debug, Clone)]
struct SelectionSnapshot {
    selections: HashMap<(usize, usize), Vec<usize>>,
    flags: HashMap<String, String>,
}

/// Overall wizard completion status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionStatus {
    pub total_steps: usize,
    pub visible_steps: usize,
    pub total_groups: usize,
    pub satisfied_groups: usize,
}

impl CompletionStatus {
    /// Completion as a fraction from 0.0 to 1.0.
    pub fn fraction(&self) -> f32 {
        if self.total_groups == 0 {
            1.0
        } else {
            self.satisfied_groups as f32 / self.total_groups as f32
        }
    }
}

/// Detailed validation hint for UI display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationHint {
    NeedExactly {
        group: String,
        required: usize,
        current: usize,
    },
    NeedAtLeast {
        group: String,
        required: usize,
        current: usize,
    },
    ExceedsMax {
        group: String,
        max: usize,
        current: usize,
    },
    NotUsableSelected {
        group: String,
        plugin: String,
    },
}

impl fmt::Display for ValidationHint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NeedExactly {
                group,
                required,
                current,
            } => write!(
                f,
                "{group}: need exactly {required}, have {current} selected"
            ),
            Self::NeedAtLeast {
                group,
                required,
                current,
            } => write!(
                f,
                "{group}: need at least {required}, have {current} selected"
            ),
            Self::ExceedsMax {
                group,
                max,
                current,
            } => write!(f, "{group}: at most {max} allowed, have {current} selected"),
            Self::NotUsableSelected { group, plugin } => {
                write!(f, "{group}: \"{plugin}\" is marked as not usable")
            }
        }
    }
}

/// A file destination that multiple sources want to write to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileConflict {
    pub destination: String,
    pub sources: Vec<FileConflictSource>,
}

/// Where a conflicting file comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileConflictSource {
    Required {
        source: String,
    },
    Plugin {
        step: usize,
        group: usize,
        plugin: usize,
        plugin_name: String,
        source: String,
    },
}

/// A plugin's flag-setting that affects another step's visibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagImpact {
    pub source_step: usize,
    pub source_group: usize,
    pub source_plugin: usize,
    pub flag_name: String,
    pub affected_step: usize,
    pub affected_step_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionError {
    OutOfBounds,
    InvalidCount { expected: &'static str, got: usize },
}

impl fmt::Display for SelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfBounds => write!(f, "plugin index out of bounds"),
            Self::InvalidCount { expected, got } => {
                write!(f, "expected {expected} selections, got {got}")
            }
        }
    }
}

impl std::error::Error for SelectionError {}

fn compute_defaults(group: &Group, type_fn: impl Fn(&Plugin) -> PluginType) -> Vec<usize> {
    match group.group_type {
        GroupType::SelectAll => (0..group.plugins.plugins.len()).collect(),
        GroupType::SelectExactlyOne | GroupType::SelectAtMostOne => group
            .plugins
            .plugins
            .iter()
            .position(|p| matches!(type_fn(p), PluginType::Required | PluginType::Recommended))
            .map(|i| vec![i])
            .unwrap_or_default(),
        GroupType::SelectAtLeastOne | GroupType::SelectAny => group
            .plugins
            .plugins
            .iter()
            .enumerate()
            .filter(|(_, p)| matches!(type_fn(p), PluginType::Required | PluginType::Recommended))
            .map(|(i, _)| i)
            .collect(),
    }
}

fn files_to_ops(files: &FileList) -> Vec<FileOperation> {
    files
        .items
        .iter()
        .map(|item| {
            let r = item.file_ref();
            FileOperation {
                source: r.source.clone(),
                destination: r.destination.clone(),
                is_folder: item.is_folder(),
                priority: r.priority,
            }
        })
        .collect()
}

fn normalize_dest(source: &str, destination: &str) -> String {
    if destination.is_empty() {
        source.to_lowercase()
    } else {
        destination.to_lowercase()
    }
}

/// Resolve a relative path case-insensitively against a base directory.
fn resolve_path_case_insensitive(base: &Path, relative: &str) -> Option<PathBuf> {
    // Normalize separators
    let parts: Vec<&str> = relative
        .split(['/', '\\'])
        .filter(|s| !s.is_empty())
        .collect();
    let mut current = base.to_path_buf();

    for part in parts {
        let entries = fs::read_dir(&current).ok()?;
        let mut found = false;
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str()
                && name.eq_ignore_ascii_case(part)
            {
                current = entry.path();
                found = true;
                break;
            }
        }
        if !found {
            return None;
        }
    }

    Some(current)
}

/// Collect all flag names referenced by a composite dependency (recursively).
fn collect_flag_names(dep: &CompositeDependency) -> Vec<String> {
    let mut names: Vec<String> = dep.flag_deps.iter().map(|f| f.flag.clone()).collect();
    for nested in &dep.nested {
        names.extend(collect_flag_names(nested));
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GroupType, ModuleConfig, PluginType};

    #[test]
    fn sort_order_ascending_sorts() {
        let mut items = vec!["Zebra", "Apple", "Mango"];
        apply_sort_order(&mut items, Some(SortOrder::Ascending), |s| s);
        assert_eq!(items, vec!["Apple", "Mango", "Zebra"]);
    }

    #[test]
    fn sort_order_descending_sorts() {
        let mut items = vec!["Apple", "Mango", "Zebra"];
        apply_sort_order(&mut items, Some(SortOrder::Descending), |s| s);
        assert_eq!(items, vec!["Zebra", "Mango", "Apple"]);
    }

    #[test]
    fn sort_order_explicit_preserves() {
        let mut items = vec!["B", "A", "C"];
        apply_sort_order(&mut items, Some(SortOrder::Explicit), |s| s);
        assert_eq!(items, vec!["B", "A", "C"]);
    }

    #[test]
    fn sort_order_none_preserves() {
        let mut items = vec!["B", "A", "C"];
        apply_sort_order(&mut items, None, |s| s);
        assert_eq!(items, vec!["B", "A", "C"]);
    }

    fn make_group(gtype: GroupType, count: usize) -> Group {
        let plugins: Vec<_> = (0..count)
            .map(|i| crate::config::Plugin {
                name: format!("P{i}"),
                description: None,
                image: None,
                type_descriptor: None,
                condition_flags: None,
                files: None,
            })
            .collect();
        Group {
            name: "G".into(),
            group_type: gtype,
            plugins: crate::config::PluginList {
                order: None,
                plugins,
            },
        }
    }

    #[test]
    fn validate_exactly_one() {
        let g = make_group(GroupType::SelectExactlyOne, 3);
        assert!(Installer::validate_selection(&g, &[0]).is_ok());
        assert!(Installer::validate_selection(&g, &[]).is_err());
        assert!(Installer::validate_selection(&g, &[0, 1]).is_err());
    }

    #[test]
    fn validate_at_most_one() {
        let g = make_group(GroupType::SelectAtMostOne, 3);
        assert!(Installer::validate_selection(&g, &[]).is_ok());
        assert!(Installer::validate_selection(&g, &[1]).is_ok());
        assert!(Installer::validate_selection(&g, &[0, 1]).is_err());
    }

    #[test]
    fn validate_at_least_one() {
        let g = make_group(GroupType::SelectAtLeastOne, 3);
        assert!(Installer::validate_selection(&g, &[]).is_err());
        assert!(Installer::validate_selection(&g, &[0]).is_ok());
        assert!(Installer::validate_selection(&g, &[0, 1, 2]).is_ok());
    }

    #[test]
    fn validate_select_all() {
        let g = make_group(GroupType::SelectAll, 2);
        assert!(Installer::validate_selection(&g, &[0]).is_err());
        assert!(Installer::validate_selection(&g, &[0, 1]).is_ok());
    }

    #[test]
    fn validate_select_any() {
        let g = make_group(GroupType::SelectAny, 3);
        assert!(Installer::validate_selection(&g, &[]).is_ok());
        assert!(Installer::validate_selection(&g, &[0, 1, 2]).is_ok());
    }

    #[test]
    fn validate_out_of_bounds() {
        let g = make_group(GroupType::SelectAny, 2);
        assert_eq!(
            Installer::validate_selection(&g, &[2]),
            Err(SelectionError::OutOfBounds)
        );
        assert_eq!(
            Installer::validate_selection(&g, &[99]),
            Err(SelectionError::OutOfBounds)
        );
    }

    fn make_group_typed(gtype: GroupType, types: Vec<PluginType>) -> Group {
        let plugins: Vec<_> = types
            .into_iter()
            .enumerate()
            .map(|(i, pt)| crate::config::Plugin {
                name: format!("P{i}"),
                description: None,
                image: None,
                type_descriptor: Some(crate::config::TypeDescriptor {
                    simple_type: Some(crate::config::SimpleType { name: pt }),
                    dependency_type: None,
                }),
                condition_flags: None,
                files: None,
            })
            .collect();
        Group {
            name: "G".into(),
            group_type: gtype,
            plugins: crate::config::PluginList {
                order: None,
                plugins,
            },
        }
    }

    #[test]
    fn defaults_exactly_one_picks_first_required() {
        let g = make_group_typed(
            GroupType::SelectExactlyOne,
            vec![
                PluginType::Optional,
                PluginType::Required,
                PluginType::Required,
            ],
        );
        assert_eq!(Installer::default_selections(&g), vec![1]);
    }

    #[test]
    fn defaults_exactly_one_picks_recommended() {
        let g = make_group_typed(
            GroupType::SelectExactlyOne,
            vec![PluginType::Optional, PluginType::Recommended],
        );
        assert_eq!(Installer::default_selections(&g), vec![1]);
    }

    #[test]
    fn defaults_exactly_one_all_optional_empty() {
        let g = make_group_typed(
            GroupType::SelectExactlyOne,
            vec![PluginType::Optional, PluginType::Optional],
        );
        assert!(Installer::default_selections(&g).is_empty());
    }

    #[test]
    fn defaults_select_all_returns_all() {
        let g = make_group_typed(
            GroupType::SelectAll,
            vec![
                PluginType::Optional,
                PluginType::Optional,
                PluginType::Optional,
            ],
        );
        assert_eq!(Installer::default_selections(&g), vec![0, 1, 2]);
    }

    #[test]
    fn defaults_any_picks_required_and_recommended() {
        let g = make_group_typed(
            GroupType::SelectAny,
            vec![
                PluginType::Optional,
                PluginType::Required,
                PluginType::Optional,
                PluginType::Recommended,
            ],
        );
        assert_eq!(Installer::default_selections(&g), vec![1, 3]);
    }

    #[test]
    fn select_clears_group_flags() {
        let xml = r#"
            <config><moduleName>T</moduleName>
            <installSteps><installStep name="S">
            <optionalFileGroups><group name="G" type="SelectExactlyOne">
            <plugins>
                <plugin name="A">
                    <conditionFlags><flag name="choice">a</flag></conditionFlags>
                    <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
                <plugin name="B">
                    <conditionFlags><flag name="choice">b</flag></conditionFlags>
                    <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
            </plugins>
            </group></optionalFileGroups>
            </installStep></installSteps></config>
        "#;
        let config = ModuleConfig::parse(xml).unwrap();
        let mut installer = Installer::new(config);

        installer.select(0, 0, vec![0]);
        assert_eq!(
            installer.context().flags.get("choice"),
            Some(&"a".to_owned())
        );

        installer.select(0, 0, vec![1]);
        assert_eq!(
            installer.context().flags.get("choice"),
            Some(&"b".to_owned())
        );
    }

    #[test]
    fn resolve_empty_no_config() {
        let xml = r#"<config><moduleName>T</moduleName></config>"#;
        let config = ModuleConfig::parse(xml).unwrap();
        let installer = Installer::new(config);
        assert!(installer.resolve().operations.is_empty());
    }

    #[test]
    fn resolve_priority_ordering() {
        let xml = r#"
            <config><moduleName>T</moduleName>
            <requiredInstallFiles>
                <file source="low.esp" destination="Data" priority="-10"/>
                <file source="high.esp" destination="Data" priority="100"/>
                <file source="mid.esp" destination="Data" priority="50"/>
            </requiredInstallFiles></config>
        "#;
        let config = ModuleConfig::parse(xml).unwrap();
        let installer = Installer::new(config);
        let plan = installer.resolve();
        let sources: Vec<&str> = plan
            .operations
            .iter()
            .map(|op| op.source.as_str())
            .collect();
        assert_eq!(sources, vec!["low.esp", "mid.esp", "high.esp"]);
    }

    #[test]
    fn resolve_skips_invalid_selection() {
        let xml = r#"
            <config><moduleName>T</moduleName>
            <installSteps><installStep name="S">
            <optionalFileGroups><group name="G" type="SelectAny">
            <plugins><plugin name="A">
                <typeDescriptor><type name="Optional"/></typeDescriptor>
                <files><file source="a.esp" destination="Data"/></files>
            </plugin></plugins>
            </group></optionalFileGroups>
            </installStep></installSteps></config>
        "#;
        let config = ModuleConfig::parse(xml).unwrap();
        let mut installer = Installer::new(config);
        installer.select(0, 0, vec![99]);
        assert!(installer.resolve().operations.is_empty());
    }

    #[test]
    fn check_deps_none_means_ok() {
        let xml = r#"<config><moduleName>T</moduleName></config>"#;
        let config = ModuleConfig::parse(xml).unwrap();
        assert!(Installer::new(config).check_dependencies());
    }

    #[test]
    fn visible_steps_empty_when_no_steps() {
        let xml = r#"<config><moduleName>T</moduleName></config>"#;
        let config = ModuleConfig::parse(xml).unwrap();
        assert!(Installer::new(config).visible_steps().is_empty());
    }

    #[test]
    fn selection_error_display() {
        assert_eq!(
            SelectionError::OutOfBounds.to_string(),
            "plugin index out of bounds"
        );
        assert_eq!(
            SelectionError::InvalidCount {
                expected: "exactly 1",
                got: 3
            }
            .to_string(),
            "expected exactly 1 selections, got 3"
        );
    }

    #[test]
    fn with_context_preserves() {
        let xml = r#"<config><moduleName>T</moduleName></config>"#;
        let config = ModuleConfig::parse(xml).unwrap();
        let mut ctx = EvalContext::new();
        ctx.set_flag("pre", "val");
        ctx.game_version = Some("1.5".into());

        let installer = Installer::with_context(config, ctx);
        assert_eq!(
            installer.context().flags.get("pre"),
            Some(&"val".to_owned())
        );
        assert_eq!(installer.context().game_version, Some("1.5".to_owned()));
    }
}
