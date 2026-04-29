use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Trait for types that can be evaluated against an installation context.
pub trait Evaluate {
    fn evaluate(&self, ctx: &EvalContext) -> bool;
}

/// Evaluate a slice of dependencies.
fn eval_iter<'a, T: Evaluate>(
    deps: &'a [T],
    ctx: &'a EvalContext,
) -> impl Iterator<Item = bool> + 'a {
    deps.iter().map(move |d| d.evaluate(ctx))
}

/// Composite dependency with AND/OR logic, potentially nested.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompositeDependency {
    #[serde(rename = "@operator", default = "default_operator")]
    pub operator: Operator,
    #[serde(rename = "fileDependency", default)]
    pub file_deps: Vec<FileDependency>,
    #[serde(rename = "flagDependency", default)]
    pub flag_deps: Vec<FlagDependency>,
    #[serde(rename = "gameDependency", default)]
    pub game_deps: Vec<GameDependency>,
    #[serde(rename = "fommDependency", default)]
    pub fomm_deps: Vec<FommDependency>,
    /// Nested composite dependencies for complex logic.
    #[serde(rename = "dependencies", default)]
    pub nested: Vec<Self>,
}

impl Evaluate for CompositeDependency {
    fn evaluate(&self, ctx: &EvalContext) -> bool {
        let mut results = eval_iter(&self.file_deps, ctx)
            .chain(eval_iter(&self.flag_deps, ctx))
            .chain(eval_iter(&self.game_deps, ctx))
            .chain(eval_iter(&self.fomm_deps, ctx))
            .chain(eval_iter(&self.nested, ctx));

        match self.operator {
            Operator::And => results.all(|v| v),
            Operator::Or => results.any(|v| v),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDependency {
    #[serde(rename = "@file")]
    pub file: String,

    #[serde(rename = "@state")]
    pub state: FileState,
}

impl Evaluate for FileDependency {
    fn evaluate(&self, ctx: &EvalContext) -> bool {
        // Case-insensitive file path matching (FOMOD is Windows-centric)
        let actual = ctx
            .file_states
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(&self.file))
            .map_or(FileState::Missing, |(_, v)| *v);
        actual == self.state
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlagDependency {
    #[serde(rename = "@flag")]
    pub flag: String,

    #[serde(rename = "@value")]
    pub value: String,
}

impl Evaluate for FlagDependency {
    fn evaluate(&self, ctx: &EvalContext) -> bool {
        ctx.flags.get(&self.flag).is_some_and(|v| v == &self.value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameDependency {
    #[serde(rename = "@version")]
    pub version: String,
}

impl Evaluate for GameDependency {
    fn evaluate(&self, ctx: &EvalContext) -> bool {
        check_version(ctx.game_version.as_deref(), &self.version)
    }
}

/// FOMM (Fallout Mod Manager) / mod manager version dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FommDependency {
    #[serde(rename = "@version")]
    pub version: String,
}

impl Evaluate for FommDependency {
    fn evaluate(&self, ctx: &EvalContext) -> bool {
        check_version(ctx.manager_version.as_deref(), &self.version)
    }
}

/// Check if `current` version satisfies `>= required`.
fn check_version(current: Option<&str>, required: &str) -> bool {
    current
        .as_ref()
        .is_some_and(|c| compare_versions(c, required))
}

/// Simple version comparison: current >= required.
fn compare_versions(current: &str, required: &str) -> bool {
    let parse = |s: &str| -> Vec<u32> { s.split('.').filter_map(|p| p.parse().ok()).collect() };
    let cur = parse(current);
    let req = parse(required);
    cur >= req
}

/// Runtime context for evaluating conditions.
#[derive(Debug, Default, Clone)]
pub struct EvalContext {
    /// Flags set by user plugin selections.
    pub flags: HashMap<String, String>,
    /// Known file states in the game directory.
    pub file_states: HashMap<String, FileState>,
    /// Current game version (e.g. "1.5.0.0").
    pub game_version: Option<String>,
    /// Current mod manager version (for `fommDependency`).
    pub manager_version: Option<String>,
}

impl EvalContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_flag(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.flags.insert(name.into(), value.into());
    }

    pub fn set_file_state(&mut self, file: impl Into<String>, state: FileState) {
        self.file_states.insert(file.into(), state);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileState {
    Active,
    Inactive,
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operator {
    And,
    Or,
}

fn default_operator() -> Operator {
    Operator::And
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_equal() {
        assert!(compare_versions("1.2.0", "1.2.0"));
    }

    #[test]
    fn version_current_greater() {
        assert!(compare_versions("1.3.0", "1.2.0"));
        assert!(compare_versions("2.0.0", "1.9.9"));
    }

    #[test]
    fn version_current_less() {
        assert!(!compare_versions("1.1.0", "1.2.0"));
        assert!(!compare_versions("0.9.9", "1.0.0"));
    }

    #[test]
    fn version_different_lengths() {
        // Shorter current < longer required (fewer components)
        assert!(!compare_versions("1.2", "1.2.0"));
        // Longer current with extra segments
        assert!(compare_versions("1.2.0.1", "1.2.0"));
    }

    #[test]
    fn version_single_segment() {
        assert!(compare_versions("2", "1"));
        assert!(compare_versions("1", "1"));
        assert!(!compare_versions("0", "1"));
    }

    #[test]
    fn version_non_numeric_segments_filtered() {
        // "alpha" is filtered out, leaving [1, 2]
        assert!(compare_versions("1.2.alpha", "1.2"));
        // Both have non-numeric -> both filtered to same
        assert!(compare_versions("1.beta.2", "1.alpha.2"));
    }

    #[test]
    fn version_empty_string() {
        // Empty parses to [] which is < any non-empty
        assert!(!compare_versions("", "1.0"));
        // Both empty -> [] >= [] is true
        assert!(compare_versions("", ""));
    }

    #[test]
    fn version_leading_zeros() {
        // "01" parses as 1, "001" as 1
        assert!(compare_versions("01.002.003", "1.2.3"));
    }

    #[test]
    fn version_gaps() {
        // "1..3" -> non-numeric empty filtered -> [1, 3]
        assert!(compare_versions("1..3", "1.3"));
    }

    #[test]
    fn eval_context_default_is_empty() {
        let ctx = EvalContext::new();
        assert!(ctx.flags.is_empty());
        assert!(ctx.file_states.is_empty());
        assert!(ctx.game_version.is_none());
        assert!(ctx.manager_version.is_none());
    }

    #[test]
    fn eval_context_set_flag() {
        let mut ctx = EvalContext::new();
        ctx.set_flag("test", "value");
        assert_eq!(ctx.flags.get("test"), Some(&"value".to_owned()));
    }

    #[test]
    fn eval_context_set_flag_overwrite() {
        let mut ctx = EvalContext::new();
        ctx.set_flag("key", "old");
        ctx.set_flag("key", "new");
        assert_eq!(ctx.flags.get("key"), Some(&"new".to_owned()));
    }

    #[test]
    fn eval_context_set_file_state() {
        let mut ctx = EvalContext::new();
        ctx.set_file_state("mod.esp", FileState::Active);
        assert_eq!(ctx.file_states.get("mod.esp"), Some(&FileState::Active));
    }

    #[test]
    fn flag_dep_matches() {
        let mut ctx = EvalContext::new();
        ctx.set_flag("flag1", "yes");
        let dep = FlagDependency {
            flag: "flag1".into(),
            value: "yes".into(),
        };
        assert!(dep.evaluate(&ctx));
    }

    #[test]
    fn flag_dep_wrong_value() {
        let mut ctx = EvalContext::new();
        ctx.set_flag("flag1", "no");
        let dep = FlagDependency {
            flag: "flag1".into(),
            value: "yes".into(),
        };
        assert!(!dep.evaluate(&ctx));
    }

    #[test]
    fn flag_dep_missing_flag() {
        let ctx = EvalContext::new();
        let dep = FlagDependency {
            flag: "missing".into(),
            value: "yes".into(),
        };
        assert!(!dep.evaluate(&ctx));
    }

    #[test]
    fn flag_dep_case_sensitive() {
        let mut ctx = EvalContext::new();
        ctx.set_flag("Flag", "On");
        let dep = FlagDependency {
            flag: "Flag".into(),
            value: "on".into(), // lowercase
        };
        assert!(!dep.evaluate(&ctx), "flag values are case-sensitive");
    }

    #[test]
    fn flag_dep_empty_value() {
        let mut ctx = EvalContext::new();
        ctx.set_flag("flag", "");
        let dep = FlagDependency {
            flag: "flag".into(),
            value: String::new(),
        };
        assert!(dep.evaluate(&ctx));
    }

    #[test]
    fn file_dep_active() {
        let mut ctx = EvalContext::new();
        ctx.set_file_state("mod.esp", FileState::Active);
        let dep = FileDependency {
            file: "mod.esp".into(),
            state: FileState::Active,
        };
        assert!(dep.evaluate(&ctx));
    }

    #[test]
    fn file_dep_inactive() {
        let mut ctx = EvalContext::new();
        ctx.set_file_state("mod.esp", FileState::Inactive);
        let dep = FileDependency {
            file: "mod.esp".into(),
            state: FileState::Inactive,
        };
        assert!(dep.evaluate(&ctx));
    }

    #[test]
    fn file_dep_missing_default() {
        let ctx = EvalContext::new();
        let dep = FileDependency {
            file: "nonexistent.esp".into(),
            state: FileState::Missing,
        };
        assert!(dep.evaluate(&ctx), "unknown files default to Missing");
    }

    #[test]
    fn file_dep_missing_but_expected_active() {
        let ctx = EvalContext::new();
        let dep = FileDependency {
            file: "nonexistent.esp".into(),
            state: FileState::Active,
        };
        assert!(!dep.evaluate(&ctx));
    }

    #[test]
    fn file_dep_case_insensitive() {
        let mut ctx = EvalContext::new();
        ctx.set_file_state("Data/Textures/Mod.dds", FileState::Active);
        let dep = FileDependency {
            file: "data/textures/mod.dds".into(),
            state: FileState::Active,
        };
        assert!(dep.evaluate(&ctx));
    }

    #[test]
    fn file_dep_wrong_state() {
        let mut ctx = EvalContext::new();
        ctx.set_file_state("mod.esp", FileState::Active);
        let dep = FileDependency {
            file: "mod.esp".into(),
            state: FileState::Inactive,
        };
        assert!(!dep.evaluate(&ctx));
    }

    #[test]
    fn game_dep_sufficient() {
        let mut ctx = EvalContext::new();
        ctx.game_version = Some("1.5.0".into());
        let dep = GameDependency {
            version: "1.5.0".into(),
        };
        assert!(dep.evaluate(&ctx));
    }

    #[test]
    fn game_dep_insufficient() {
        let mut ctx = EvalContext::new();
        ctx.game_version = Some("1.4.0".into());
        let dep = GameDependency {
            version: "1.5.0".into(),
        };
        assert!(!dep.evaluate(&ctx));
    }

    #[test]
    fn game_dep_no_version_set() {
        let ctx = EvalContext::new();
        let dep = GameDependency {
            version: "1.0.0".into(),
        };
        assert!(!dep.evaluate(&ctx));
    }

    #[test]
    fn fomm_dep_sufficient() {
        let mut ctx = EvalContext::new();
        ctx.manager_version = Some("2.0.0".into());
        let dep = FommDependency {
            version: "1.0.0".into(),
        };
        assert!(dep.evaluate(&ctx));
    }

    #[test]
    fn fomm_dep_no_version_set() {
        let ctx = EvalContext::new();
        let dep = FommDependency {
            version: "1.0.0".into(),
        };
        assert!(!dep.evaluate(&ctx));
    }

    fn make_flag_dep(flag: &str, value: &str) -> FlagDependency {
        FlagDependency {
            flag: flag.into(),
            value: value.into(),
        }
    }

    fn make_composite(op: Operator, flag_deps: Vec<FlagDependency>) -> CompositeDependency {
        CompositeDependency {
            operator: op,
            file_deps: Vec::new(),
            flag_deps,
            game_deps: Vec::new(),
            fomm_deps: Vec::new(),
            nested: Vec::new(),
        }
    }

    #[test]
    fn composite_and_all_true() {
        let mut ctx = EvalContext::new();
        ctx.set_flag("a", "1");
        ctx.set_flag("b", "2");
        let comp = make_composite(
            Operator::And,
            vec![make_flag_dep("a", "1"), make_flag_dep("b", "2")],
        );
        assert!(comp.evaluate(&ctx));
    }

    #[test]
    fn composite_and_one_false() {
        let mut ctx = EvalContext::new();
        ctx.set_flag("a", "1");
        let comp = make_composite(
            Operator::And,
            vec![make_flag_dep("a", "1"), make_flag_dep("b", "2")],
        );
        assert!(!comp.evaluate(&ctx));
    }

    #[test]
    fn composite_or_one_true() {
        let mut ctx = EvalContext::new();
        ctx.set_flag("a", "1");
        let comp = make_composite(
            Operator::Or,
            vec![make_flag_dep("a", "1"), make_flag_dep("b", "2")],
        );
        assert!(comp.evaluate(&ctx));
    }

    #[test]
    fn composite_or_none_true() {
        let ctx = EvalContext::new();
        let comp = make_composite(
            Operator::Or,
            vec![make_flag_dep("a", "1"), make_flag_dep("b", "2")],
        );
        assert!(!comp.evaluate(&ctx));
    }

    #[test]
    fn composite_and_empty_is_true() {
        let ctx = EvalContext::new();
        let comp = make_composite(Operator::And, Vec::new());
        assert!(comp.evaluate(&ctx), "AND over empty set is vacuously true");
    }

    #[test]
    fn composite_or_empty_is_false() {
        let ctx = EvalContext::new();
        let comp = make_composite(Operator::Or, Vec::new());
        assert!(
            !comp.evaluate(&ctx),
            "OR over empty set is false (no element satisfies)"
        );
    }

    #[test]
    fn composite_nested_and_or() {
        // (a=1 AND (b=2 OR c=3))
        let mut ctx = EvalContext::new();
        ctx.set_flag("a", "1");
        ctx.set_flag("c", "3");

        let inner = make_composite(
            Operator::Or,
            vec![make_flag_dep("b", "2"), make_flag_dep("c", "3")],
        );
        let outer = CompositeDependency {
            operator: Operator::And,
            flag_deps: vec![make_flag_dep("a", "1")],
            nested: vec![inner],
            file_deps: Vec::new(),
            game_deps: Vec::new(),
            fomm_deps: Vec::new(),
        };
        assert!(outer.evaluate(&ctx));
    }

    #[test]
    fn composite_nested_fails_outer() {
        // (a=WRONG AND (b=2 OR c=3))
        let mut ctx = EvalContext::new();
        ctx.set_flag("a", "wrong");
        ctx.set_flag("c", "3");

        let inner = make_composite(
            Operator::Or,
            vec![make_flag_dep("b", "2"), make_flag_dep("c", "3")],
        );
        let outer = CompositeDependency {
            operator: Operator::And,
            flag_deps: vec![make_flag_dep("a", "1")],
            nested: vec![inner],
            file_deps: Vec::new(),
            game_deps: Vec::new(),
            fomm_deps: Vec::new(),
        };
        assert!(!outer.evaluate(&ctx));
    }

    #[test]
    fn composite_mixed_dep_types() {
        let mut ctx = EvalContext::new();
        ctx.set_flag("flag", "yes");
        ctx.set_file_state("mod.esp", FileState::Active);
        ctx.game_version = Some("1.5.0".into());

        let comp = CompositeDependency {
            operator: Operator::And,
            flag_deps: vec![make_flag_dep("flag", "yes")],
            file_deps: vec![FileDependency {
                file: "mod.esp".into(),
                state: FileState::Active,
            }],
            game_deps: vec![GameDependency {
                version: "1.5.0".into(),
            }],
            fomm_deps: Vec::new(),
            nested: Vec::new(),
        };
        assert!(comp.evaluate(&ctx));
    }

    #[test]
    fn composite_mixed_one_fails() {
        let mut ctx = EvalContext::new();
        ctx.set_flag("flag", "yes");
        // file not set -> Missing, but we require Active

        let comp = CompositeDependency {
            operator: Operator::And,
            flag_deps: vec![make_flag_dep("flag", "yes")],
            file_deps: vec![FileDependency {
                file: "mod.esp".into(),
                state: FileState::Active,
            }],
            game_deps: Vec::new(),
            fomm_deps: Vec::new(),
            nested: Vec::new(),
        };
        assert!(!comp.evaluate(&ctx));
    }

    #[test]
    fn composite_deeply_nested() {
        // 5 levels deep: AND(OR(AND(OR(flag=yes))))
        let mut ctx = EvalContext::new();
        ctx.set_flag("deep", "yes");

        let level4 = make_composite(Operator::Or, vec![make_flag_dep("deep", "yes")]);
        let level3 = CompositeDependency {
            operator: Operator::And,
            nested: vec![level4],
            flag_deps: Vec::new(),
            file_deps: Vec::new(),
            game_deps: Vec::new(),
            fomm_deps: Vec::new(),
        };
        let level2 = CompositeDependency {
            operator: Operator::Or,
            nested: vec![level3],
            flag_deps: Vec::new(),
            file_deps: Vec::new(),
            game_deps: Vec::new(),
            fomm_deps: Vec::new(),
        };
        let level1 = CompositeDependency {
            operator: Operator::And,
            nested: vec![level2],
            flag_deps: Vec::new(),
            file_deps: Vec::new(),
            game_deps: Vec::new(),
            fomm_deps: Vec::new(),
        };
        assert!(level1.evaluate(&ctx));
    }
}
