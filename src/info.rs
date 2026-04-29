use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Metadata from a FOMOD `info.xml` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename = "fomod")]
pub struct FomodInfo {
    #[serde(rename = "Name")]
    pub name: Option<String>,
    #[serde(rename = "Author")]
    pub author: Option<String>,
    #[serde(rename = "Version")]
    pub version: Option<String>,
    #[serde(rename = "Description")]
    pub description: Option<String>,
    #[serde(rename = "Website")]
    pub website: Option<String>,
    #[serde(rename = "Id")]
    pub id: Option<String>,
}

impl FomodInfo {
    pub fn parse(xml: &str) -> Result<Self> {
        quick_xml::de::from_str(xml).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_all_fields() {
        let xml = r#"
            <fomod>
                <Name>Test Mod</Name>
                <Author>Author</Author>
                <Version>1.0</Version>
                <Description>Desc</Description>
                <Website>https://example.com</Website>
                <Id>12345</Id>
            </fomod>
        "#;
        let info = FomodInfo::parse(xml).unwrap();
        assert_eq!(info.name.as_deref(), Some("Test Mod"));
        assert_eq!(info.author.as_deref(), Some("Author"));
        assert_eq!(info.version.as_deref(), Some("1.0"));
        assert_eq!(info.description.as_deref(), Some("Desc"));
        assert_eq!(info.website.as_deref(), Some("https://example.com"));
        assert_eq!(info.id.as_deref(), Some("12345"));
    }

    #[test]
    fn parse_minimal_empty_fomod() {
        let xml = "<fomod></fomod>";
        let info = FomodInfo::parse(xml).unwrap();
        assert!(info.name.is_none());
        assert!(info.author.is_none());
        assert!(info.version.is_none());
        assert!(info.description.is_none());
        assert!(info.website.is_none());
        assert!(info.id.is_none());
    }

    #[test]
    fn parse_partial_fields() {
        let xml = r#"
            <fomod>
                <Name>Partial</Name>
                <Version>2.0</Version>
            </fomod>
        "#;
        let info = FomodInfo::parse(xml).unwrap();
        assert_eq!(info.name.as_deref(), Some("Partial"));
        assert!(info.author.is_none());
        assert_eq!(info.version.as_deref(), Some("2.0"));
        assert!(info.description.is_none());
    }

    #[test]
    fn parse_empty_name_field() {
        let xml = r#"
            <fomod>
                <Name></Name>
            </fomod>
        "#;
        let info = FomodInfo::parse(xml).unwrap();
        assert!(info.name.is_some());
    }

    #[test]
    fn parse_invalid_xml_fails() {
        let xml = "not xml at all";
        assert!(FomodInfo::parse(xml).is_err());
    }

    #[test]
    fn parse_unicode_content() {
        let xml = r#"
            <fomod>
                <Name>日本語MOD</Name>
                <Author>Ünïcödé</Author>
                <Description>Описание мода</Description>
            </fomod>
        "#;
        let info = FomodInfo::parse(xml).unwrap();
        assert_eq!(info.name.as_deref(), Some("日本語MOD"));
        assert_eq!(info.author.as_deref(), Some("Ünïcödé"));
        assert_eq!(info.description.as_deref(), Some("Описание мода"));
    }
}
