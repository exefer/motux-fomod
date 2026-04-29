use serde::{Deserialize, Serialize};

use crate::condition::{CompositeDependency, EvalContext, Evaluate};
use crate::error::Result;

/// Root element of a FOMOD `ModuleConfig.xml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename = "config")]
pub struct ModuleConfig {
    #[serde(rename = "moduleName")]
    pub module_name: ModuleName,
    #[serde(rename = "moduleImage")]
    pub module_image: Option<ModuleImage>,
    #[serde(rename = "moduleDependencies")]
    pub module_dependencies: Option<CompositeDependency>,
    #[serde(rename = "requiredInstallFiles")]
    pub required_install_files: Option<FileList>,
    #[serde(rename = "installSteps")]
    pub install_steps: Option<InstallSteps>,
    #[serde(rename = "conditionalFileInstalls")]
    pub conditional_file_installs: Option<ConditionalFileInstalls>,
}

impl ModuleConfig {
    pub fn parse(xml: &str) -> Result<Self> {
        quick_xml::de::from_str(xml).map_err(Into::into)
    }
}

/// Module display name with optional positioning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleName {
    #[serde(rename = "@position")]
    pub position: Option<NamePosition>,
    #[serde(rename = "$text")]
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NamePosition {
    Left,
    Right,
    RightOfImage,
}

/// Header/banner image for the installer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleImage {
    #[serde(rename = "@path")]
    pub path: String,
    #[serde(rename = "@showImage", default = "default_true")]
    pub show_image: bool,
    #[serde(rename = "@showFade", default = "default_true")]
    pub show_fade: bool,
    #[serde(rename = "@height", default = "default_neg_one")]
    pub height: i32,
}

/// Ordered sequence of installation steps (pages).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallSteps {
    #[serde(rename = "@order")]
    pub order: Option<SortOrder>,
    #[serde(rename = "installStep", default)]
    pub steps: Vec<InstallStep>,
}

/// A single page presented to the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallStep {
    #[serde(rename = "@name")]
    pub name: String,
    pub visible: Option<CompositeDependency>,
    #[serde(rename = "optionalFileGroups")]
    pub optional_file_groups: Option<GroupList>,
}

/// Container for option groups within a step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupList {
    #[serde(rename = "@order")]
    pub order: Option<SortOrder>,
    #[serde(rename = "group", default)]
    pub groups: Vec<Group>,
}

/// A group of related installation options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@type")]
    pub group_type: GroupType,
    #[serde(rename = "plugins")]
    pub plugins: PluginList,
}

/// Container for plugins within a group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginList {
    #[serde(rename = "@order")]
    pub order: Option<SortOrder>,
    #[serde(rename = "plugin", default)]
    pub plugins: Vec<Plugin>,
}

/// An individual installation option the user can select.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plugin {
    #[serde(rename = "@name")]
    pub name: String,
    pub description: Option<String>,
    pub image: Option<PluginImage>,
    #[serde(rename = "typeDescriptor")]
    pub type_descriptor: Option<TypeDescriptor>,
    #[serde(rename = "conditionFlags")]
    pub condition_flags: Option<ConditionFlagList>,
    pub files: Option<FileList>,
}

impl Plugin {
    /// Resolved plugin type using static information only, defaulting to `Optional`.
    ///
    /// For `dependencyType` descriptors, returns the default type without
    /// evaluating patterns. Use [`plugin_type_in_context`](Self::plugin_type_in_context)
    /// when runtime condition evaluation is needed.
    pub fn plugin_type(&self) -> PluginType {
        self.type_descriptor
            .as_ref()
            .map_or(PluginType::Optional, |td| td.resolved_type())
    }

    /// Resolved plugin type with runtime condition evaluation.
    ///
    /// Evaluates `dependencyType` patterns against the provided context,
    /// falling back to the default type if no pattern matches.
    pub fn plugin_type_in_context(&self, ctx: &EvalContext) -> PluginType {
        self.type_descriptor
            .as_ref()
            .map_or(PluginType::Optional, |td| td.resolved_type_in_context(ctx))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginImage {
    #[serde(rename = "@path")]
    pub path: String,
}

/// Describes the selection type of a plugin. Supports both simple
/// `<type name="..."/>` and conditional `<dependencyType>` forms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeDescriptor {
    #[serde(rename = "type")]
    pub simple_type: Option<SimpleType>,
    #[serde(rename = "dependencyType")]
    pub dependency_type: Option<DependencyType>,
}

impl TypeDescriptor {
    /// Static type resolution (no condition evaluation).
    pub fn resolved_type(&self) -> PluginType {
        self.resolve_with(|_| false)
    }

    /// Resolve type by evaluating `dependencyType` patterns against context.
    pub fn resolved_type_in_context(&self, ctx: &EvalContext) -> PluginType {
        self.resolve_with(|dep| dep.evaluate(ctx))
    }

    fn resolve_with(&self, eval: impl Fn(&CompositeDependency) -> bool) -> PluginType {
        if let Some(st) = &self.simple_type {
            return st.name;
        }
        if let Some(dt) = &self.dependency_type {
            if let Some(patterns) = &dt.patterns {
                for pattern in &patterns.patterns {
                    if eval(&pattern.dependencies) {
                        return pattern.plugin_type.name;
                    }
                }
            }
            return dt.default_type.name;
        }
        PluginType::Optional
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleType {
    #[serde(rename = "@name")]
    pub name: PluginType,
}

/// Conditional type descriptor - type depends on runtime conditions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyType {
    #[serde(rename = "defaultType")]
    pub default_type: SimpleType,
    #[serde(rename = "patterns")]
    pub patterns: Option<TypePatterns>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypePatterns {
    #[serde(rename = "pattern", default)]
    pub patterns: Vec<TypePattern>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypePattern {
    pub dependencies: CompositeDependency,
    #[serde(rename = "type")]
    pub plugin_type: SimpleType,
}

/// Flags set when a plugin is selected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionFlagList {
    #[serde(rename = "flag", default)]
    pub flags: Vec<ConditionFlag>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionFlag {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "$text")]
    pub value: String,
}

/// Pattern-based conditional file installations evaluated after all steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionalFileInstalls {
    pub patterns: ConditionalPatterns,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionalPatterns {
    #[serde(rename = "pattern", default)]
    pub patterns: Vec<ConditionalPattern>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionalPattern {
    pub dependencies: CompositeDependency,
    pub files: FileList,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileList {
    #[serde(rename = "$value", default)]
    pub items: Vec<FileItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileItem {
    File(FileRef),
    Folder(FileRef),
}

impl FileItem {
    pub fn file_ref(&self) -> &FileRef {
        match self {
            Self::File(r) | Self::Folder(r) => r,
        }
    }

    pub fn is_folder(&self) -> bool {
        matches!(self, Self::Folder(_))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRef {
    #[serde(rename = "@source")]
    pub source: String,
    #[serde(rename = "@destination", default)]
    pub destination: String,
    #[serde(rename = "@priority", default)]
    pub priority: i32,
    #[serde(rename = "@alwaysInstall", default)]
    pub always_install: bool,
    #[serde(rename = "@installIfUsable", default)]
    pub install_if_usable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupType {
    SelectExactlyOne,
    SelectAtMostOne,
    SelectAtLeastOne,
    SelectAll,
    SelectAny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginType {
    Required,
    Recommended,
    Optional,
    CouldBeUsable,
    NotUsable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortOrder {
    Explicit,
    Ascending,
    Descending,
}

fn default_true() -> bool {
    true
}

fn default_neg_one() -> i32 {
    -1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_config() {
        let xml = r#"<config><moduleName>Test</moduleName></config>"#;
        let config = ModuleConfig::parse(xml).unwrap();
        assert_eq!(config.module_name.value, "Test");
        assert!(config.module_image.is_none());
        assert!(config.module_dependencies.is_none());
        assert!(config.required_install_files.is_none());
        assert!(config.install_steps.is_none());
        assert!(config.conditional_file_installs.is_none());
    }

    #[test]
    fn parse_empty_module_name_fails() {
        let xml = r#"<config><moduleName></moduleName></config>"#;
        assert!(ModuleConfig::parse(xml).is_err());
    }

    #[test]
    fn parse_module_name_with_position() {
        let xml = r#"<config><moduleName position="Left">Test</moduleName></config>"#;
        let config = ModuleConfig::parse(xml).unwrap();
        assert_eq!(config.module_name.position, Some(NamePosition::Left));

        let xml = r#"<config><moduleName position="RightOfImage">Test</moduleName></config>"#;
        let config = ModuleConfig::parse(xml).unwrap();
        assert_eq!(
            config.module_name.position,
            Some(NamePosition::RightOfImage)
        );
    }

    #[test]
    fn parse_invalid_xml_fails() {
        assert!(ModuleConfig::parse("not xml").is_err());
        assert!(ModuleConfig::parse("").is_err());
        assert!(ModuleConfig::parse("<config></config>").is_err());
    }

    #[test]
    fn parse_module_image_defaults() {
        let xml = r#"
            <config>
                <moduleName>Test</moduleName>
                <moduleImage path="img.png"/>
            </config>
        "#;
        let config = ModuleConfig::parse(xml).unwrap();
        let img = config.module_image.unwrap();
        assert_eq!(img.path, "img.png");
        assert!(img.show_image);
        assert!(img.show_fade);
        assert_eq!(img.height, -1);
    }

    #[test]
    fn parse_module_image_custom() {
        let xml = r#"
            <config>
                <moduleName>Test</moduleName>
                <moduleImage path="img.png" showImage="false" showFade="false" height="200"/>
            </config>
        "#;
        let config = ModuleConfig::parse(xml).unwrap();
        let img = config.module_image.unwrap();
        assert!(!img.show_image);
        assert!(!img.show_fade);
        assert_eq!(img.height, 200);
    }

    #[test]
    fn parse_all_group_types() {
        for (type_str, expected) in [
            ("SelectExactlyOne", GroupType::SelectExactlyOne),
            ("SelectAtMostOne", GroupType::SelectAtMostOne),
            ("SelectAtLeastOne", GroupType::SelectAtLeastOne),
            ("SelectAll", GroupType::SelectAll),
            ("SelectAny", GroupType::SelectAny),
        ] {
            let xml = format!(
                r#"<config><moduleName>T</moduleName>
                <installSteps><installStep name="S">
                <optionalFileGroups><group name="G" type="{type_str}">
                <plugins><plugin name="P"><typeDescriptor><type name="Optional"/></typeDescriptor></plugin></plugins>
                </group></optionalFileGroups>
                </installStep></installSteps></config>"#
            );
            let config = ModuleConfig::parse(&xml).unwrap();
            let group = &config.install_steps.as_ref().unwrap().steps[0]
                .optional_file_groups
                .as_ref()
                .unwrap()
                .groups[0];
            assert_eq!(group.group_type, expected, "Failed for {type_str}");
        }
    }

    #[test]
    fn parse_all_plugin_types() {
        for (type_str, expected) in [
            ("Required", PluginType::Required),
            ("Recommended", PluginType::Recommended),
            ("Optional", PluginType::Optional),
            ("CouldBeUsable", PluginType::CouldBeUsable),
            ("NotUsable", PluginType::NotUsable),
        ] {
            let xml = format!(
                r#"<config><moduleName>T</moduleName>
                <installSteps><installStep name="S">
                <optionalFileGroups><group name="G" type="SelectAny">
                <plugins><plugin name="P"><typeDescriptor><type name="{type_str}"/></typeDescriptor></plugin></plugins>
                </group></optionalFileGroups>
                </installStep></installSteps></config>"#
            );
            let config = ModuleConfig::parse(&xml).unwrap();
            let plugin = &config.install_steps.as_ref().unwrap().steps[0]
                .optional_file_groups
                .as_ref()
                .unwrap()
                .groups[0]
                .plugins
                .plugins[0];
            assert_eq!(plugin.plugin_type(), expected, "Failed for {type_str}");
        }
    }

    #[test]
    fn plugin_type_defaults_to_optional() {
        let xml = r#"
            <config><moduleName>T</moduleName>
            <installSteps><installStep name="S">
            <optionalFileGroups><group name="G" type="SelectAny">
            <plugins><plugin name="P"></plugin></plugins>
            </group></optionalFileGroups>
            </installStep></installSteps></config>
        "#;
        let config = ModuleConfig::parse(xml).unwrap();
        let plugin = &config.install_steps.as_ref().unwrap().steps[0]
            .optional_file_groups
            .as_ref()
            .unwrap()
            .groups[0]
            .plugins
            .plugins[0];
        assert_eq!(plugin.plugin_type(), PluginType::Optional);
    }

    #[test]
    fn type_descriptor_simple_takes_precedence() {
        // If both simple_type and dependency_type exist, simple_type wins
        let td = TypeDescriptor {
            simple_type: Some(SimpleType {
                name: PluginType::Required,
            }),
            dependency_type: Some(DependencyType {
                default_type: SimpleType {
                    name: PluginType::NotUsable,
                },
                patterns: None,
            }),
        };
        assert_eq!(td.resolved_type(), PluginType::Required);

        let ctx = EvalContext::default();
        assert_eq!(td.resolved_type_in_context(&ctx), PluginType::Required);
    }

    #[test]
    fn type_descriptor_dependency_default() {
        let td = TypeDescriptor {
            simple_type: None,
            dependency_type: Some(DependencyType {
                default_type: SimpleType {
                    name: PluginType::NotUsable,
                },
                patterns: None,
            }),
        };
        assert_eq!(td.resolved_type(), PluginType::NotUsable);
    }

    #[test]
    fn type_descriptor_none_defaults_optional() {
        let td = TypeDescriptor {
            simple_type: None,
            dependency_type: None,
        };
        assert_eq!(td.resolved_type(), PluginType::Optional);
    }

    #[test]
    fn file_item_is_folder() {
        let file = FileItem::File(FileRef {
            source: "a.esp".into(),
            destination: String::new(),
            priority: 0,
            always_install: false,
            install_if_usable: false,
        });
        assert!(!file.is_folder());

        let folder = FileItem::Folder(FileRef {
            source: "dir".into(),
            destination: String::new(),
            priority: 0,
            always_install: false,
            install_if_usable: false,
        });
        assert!(folder.is_folder());
    }

    #[test]
    fn file_item_file_ref() {
        let file = FileItem::File(FileRef {
            source: "a.esp".into(),
            destination: "Data".into(),
            priority: 5,
            always_install: true,
            install_if_usable: false,
        });
        let r = file.file_ref();
        assert_eq!(r.source, "a.esp");
        assert_eq!(r.destination, "Data");
        assert_eq!(r.priority, 5);
        assert!(r.always_install);
    }

    #[test]
    fn parse_file_ref_defaults() {
        let xml = r#"
            <config><moduleName>T</moduleName>
            <requiredInstallFiles>
                <file source="test.esp"/>
            </requiredInstallFiles></config>
        "#;
        let config = ModuleConfig::parse(xml).unwrap();
        let item = &config.required_install_files.as_ref().unwrap().items[0];
        let r = item.file_ref();
        assert_eq!(r.source, "test.esp");
        assert_eq!(r.destination, "");
        assert_eq!(r.priority, 0);
        assert!(!r.always_install);
        assert!(!r.install_if_usable);
    }

    #[test]
    fn parse_file_and_folder_mix() {
        let xml = r#"
            <config><moduleName>T</moduleName>
            <requiredInstallFiles>
                <file source="a.esp" destination="Data"/>
                <folder source="meshes" destination="Data/meshes"/>
                <file source="b.esp" destination="Data" priority="10"/>
            </requiredInstallFiles></config>
        "#;
        let config = ModuleConfig::parse(xml).unwrap();
        let items = &config.required_install_files.as_ref().unwrap().items;
        assert_eq!(items.len(), 3);
        assert!(!items[0].is_folder());
        assert!(items[1].is_folder());
        assert!(!items[2].is_folder());
        assert_eq!(items[2].file_ref().priority, 10);
    }

    #[test]
    fn parse_sort_orders() {
        for (order_str, expected) in [
            ("Explicit", SortOrder::Explicit),
            ("Ascending", SortOrder::Ascending),
            ("Descending", SortOrder::Descending),
        ] {
            let xml = format!(
                r#"<config><moduleName>T</moduleName>
                <installSteps order="{order_str}">
                <installStep name="S">
                <optionalFileGroups><group name="G" type="SelectAny">
                <plugins><plugin name="P"><typeDescriptor><type name="Optional"/></typeDescriptor></plugin></plugins>
                </group></optionalFileGroups>
                </installStep></installSteps></config>"#
            );
            let config = ModuleConfig::parse(&xml).unwrap();
            assert_eq!(
                config.install_steps.as_ref().unwrap().order,
                Some(expected),
                "Failed for {order_str}"
            );
        }
    }

    #[test]
    fn parse_empty_required_files() {
        let xml = r#"
            <config><moduleName>T</moduleName>
            <requiredInstallFiles></requiredInstallFiles></config>
        "#;
        let config = ModuleConfig::parse(xml).unwrap();
        assert!(
            config
                .required_install_files
                .as_ref()
                .unwrap()
                .items
                .is_empty()
        );
    }

    #[test]
    fn parse_empty_install_steps() {
        let xml = r#"
            <config><moduleName>T</moduleName>
            <installSteps></installSteps></config>
        "#;
        let config = ModuleConfig::parse(xml).unwrap();
        assert!(config.install_steps.as_ref().unwrap().steps.is_empty());
    }

    #[test]
    fn parse_unicode_names() {
        let xml = r#"
            <config><moduleName>日本語MOD</moduleName>
            <installSteps><installStep name="ステップ1">
            <optionalFileGroups><group name="グループ" type="SelectAny">
            <plugins><plugin name="プラグイン"><typeDescriptor><type name="Optional"/></typeDescriptor></plugin></plugins>
            </group></optionalFileGroups>
            </installStep></installSteps></config>
        "#;
        let config = ModuleConfig::parse(xml).unwrap();
        assert_eq!(config.module_name.value, "日本語MOD");
        let step = &config.install_steps.as_ref().unwrap().steps[0];
        assert_eq!(step.name, "ステップ1");
    }

    #[test]
    fn parse_conditional_file_installs() {
        let xml = r#"
            <config><moduleName>T</moduleName>
            <conditionalFileInstalls><patterns>
                <pattern>
                    <dependencies operator="And">
                        <flagDependency flag="f" value="v"/>
                    </dependencies>
                    <files><file source="a.esp" destination="Data"/></files>
                </pattern>
                <pattern>
                    <dependencies operator="Or">
                        <flagDependency flag="x" value="1"/>
                        <flagDependency flag="y" value="2"/>
                    </dependencies>
                    <files>
                        <folder source="dir" destination="Data/dir"/>
                    </files>
                </pattern>
            </patterns></conditionalFileInstalls></config>
        "#;
        let config = ModuleConfig::parse(xml).unwrap();
        let cfi = config.conditional_file_installs.as_ref().unwrap();
        assert_eq!(cfi.patterns.patterns.len(), 2);
        assert_eq!(cfi.patterns.patterns[0].files.items.len(), 1);
        assert_eq!(cfi.patterns.patterns[1].files.items.len(), 1);
    }

    #[test]
    fn parse_condition_flags() {
        let xml = r#"
            <config><moduleName>T</moduleName>
            <installSteps><installStep name="S">
            <optionalFileGroups><group name="G" type="SelectAny">
            <plugins><plugin name="P">
                <conditionFlags>
                    <flag name="flag_a">value_a</flag>
                    <flag name="flag_b">value_b</flag>
                </conditionFlags>
                <typeDescriptor><type name="Optional"/></typeDescriptor>
            </plugin></plugins>
            </group></optionalFileGroups>
            </installStep></installSteps></config>
        "#;
        let config = ModuleConfig::parse(xml).unwrap();
        let plugin = &config.install_steps.as_ref().unwrap().steps[0]
            .optional_file_groups
            .as_ref()
            .unwrap()
            .groups[0]
            .plugins
            .plugins[0];
        let flags = plugin.condition_flags.as_ref().unwrap();
        assert_eq!(flags.flags.len(), 2);
        assert_eq!(flags.flags[0].name, "flag_a");
        assert_eq!(flags.flags[0].value, "value_a");
        assert_eq!(flags.flags[1].name, "flag_b");
        assert_eq!(flags.flags[1].value, "value_b");
    }
}
