//! Turn `{% tagref %}` targets that are not in this PDF into website links.
//!
//! Scope is the rendered tree (conditionals already resolved), so a tag
//! that was dropped from the PDF is not an internal destination. The
//! caller supplies `tag id → { url, title }` from the site catalog.

use std::collections::{HashMap, HashSet};

use markdoc::types::{RenderableTreeNode, Scalar, Tag};

use super::inline::anchor_id_attr;

/// Website destination for a `{% tag %}` that is not in the PDF.
#[derive(Debug, Clone, Default)]
pub struct ExternalTagref {
    pub url: String,
    pub title: String,
}

/// Replace out-of-scope `{% tagref %}` nodes with `<a href>` links.
///
/// Tagrefs whose target `{% tag %}` is in `root` are left alone so they
/// stay internal PDF destinations. Tagrefs with no catalog entry are
/// left alone as well.
pub fn externalize_out_of_scope_tagrefs(
    root: &RenderableTreeNode,
    external: &HashMap<String, ExternalTagref>,
) -> RenderableTreeNode {
    if external.is_empty() {
        return root.clone();
    }

    let mut scope = HashSet::new();
    collect_tag_ids(root, &mut scope);
    let mut missing = HashSet::new();
    let rewritten = rewrite(root, &scope, external, &mut missing);
    for id in missing {
        eprintln!("warning: tagref {id:?} is not in this PDF and has no page on the website");
    }
    rewritten
}

fn collect_tag_ids(node: &RenderableTreeNode, scope: &mut HashSet<String>) {
    let RenderableTreeNode::Tag(tag) = node else {
        return;
    };
    if tag.name == "tag"
        && let Some(id) = anchor_id_attr(tag)
    {
        scope.insert(id);
    }
    for child in &tag.children {
        collect_tag_ids(child, scope);
    }
}

fn rewrite(
    node: &RenderableTreeNode,
    scope: &HashSet<String>,
    external: &HashMap<String, ExternalTagref>,
    missing: &mut HashSet<String>,
) -> RenderableTreeNode {
    let RenderableTreeNode::Tag(tag) = node else {
        return node.clone();
    };

    if tag.name == "tagref"
        && let Some(id) = anchor_id_attr(tag)
        && !scope.contains(&id)
    {
        if let Some(dest) = external.get(&id).filter(|dest| !dest.url.is_empty()) {
            // Print the full URL. A heading title disappears on paper.
            return link_node(&dest.url);
        }
        missing.insert(id);
    }

    let children = tag
        .children
        .iter()
        .map(|child| rewrite(child, scope, external, missing))
        .collect();
    RenderableTreeNode::Tag(Box::new(Tag {
        name: tag.name.clone(),
        attributes: tag.attributes.clone(),
        children,
    }))
}

fn link_node(url: &str) -> RenderableTreeNode {
    let mut attributes = markdoc::IndexMap::new();
    attributes.insert("href".to_string(), Scalar::String(url.to_string()));
    attributes.insert("title".to_string(), Scalar::String(url.to_string()));
    RenderableTreeNode::Tag(Box::new(Tag {
        name: "a".to_string(),
        attributes,
        children: vec![RenderableTreeNode::Scalar(Scalar::String(url.to_string()))],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string_attr(key: &str, value: &str) -> (String, Scalar) {
        (key.to_string(), Scalar::String(value.to_string()))
    }

    fn tag(
        name: &str,
        attrs: Vec<(String, Scalar)>,
        children: Vec<RenderableTreeNode>,
    ) -> RenderableTreeNode {
        RenderableTreeNode::Tag(Box::new(Tag {
            name: name.to_string(),
            attributes: attrs.into_iter().collect(),
            children,
        }))
    }

    fn href_of(node: &RenderableTreeNode) -> Option<String> {
        let RenderableTreeNode::Tag(tag) = node else {
            return None;
        };
        match tag.attributes.get("href") {
            Some(Scalar::String(href)) => Some(href.clone()),
            _ => None,
        }
    }

    fn text_of(node: &RenderableTreeNode) -> String {
        let RenderableTreeNode::Tag(tag) = node else {
            return String::new();
        };
        tag.children
            .iter()
            .filter_map(|child| match child {
                RenderableTreeNode::Scalar(Scalar::String(text)) => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn in_scope_tagref_stays_internal() {
        let root = tag(
            "p",
            vec![],
            vec![
                tag("tag", vec![string_attr("id", "brake")], vec![]),
                tag("tagref", vec![string_attr("id", "brake")], vec![]),
            ],
        );
        let mut external = HashMap::new();
        external.insert(
            "brake".to_string(),
            ExternalTagref {
                url: "https://docs.example/brake".into(),
                title: "Brake".into(),
            },
        );
        let rewritten = externalize_out_of_scope_tagrefs(&root, &external);
        let RenderableTreeNode::Tag(paragraph) = &rewritten else {
            panic!("expected paragraph");
        };
        assert_eq!(paragraph.children[1].tag_name(), Some("tagref"));
    }

    #[test]
    fn out_of_scope_tagref_becomes_website_link() {
        let root = tag(
            "p",
            vec![],
            vec![tag(
                "tagref",
                vec![string_attr("primary", "modbus")],
                vec![],
            )],
        );
        let mut external = HashMap::new();
        external.insert(
            "modbus".to_string(),
            ExternalTagref {
                url: "https://docs.example/modbus#modbus".into(),
                title: "Modbus".into(),
            },
        );
        let rewritten = externalize_out_of_scope_tagrefs(&root, &external);
        let RenderableTreeNode::Tag(paragraph) = &rewritten else {
            panic!("expected paragraph");
        };
        let link = &paragraph.children[0];
        assert_eq!(link.tag_name(), Some("a"));
        assert_eq!(
            href_of(link).as_deref(),
            Some("https://docs.example/modbus#modbus")
        );
        assert_eq!(text_of(link), "https://docs.example/modbus#modbus");
    }

    #[test]
    fn printed_text_is_the_full_url_even_with_text_attribute() {
        let root = tag(
            "p",
            vec![],
            vec![tag(
                "tagref",
                vec![
                    string_attr("id", "modbus"),
                    string_attr("text", "the Modbus page"),
                ],
                vec![],
            )],
        );
        let mut external = HashMap::new();
        external.insert(
            "modbus".to_string(),
            ExternalTagref {
                url: "https://docs.example/modbus#modbus".into(),
                title: "Modbus".into(),
            },
        );
        let rewritten = externalize_out_of_scope_tagrefs(&root, &external);
        let RenderableTreeNode::Tag(paragraph) = &rewritten else {
            panic!("expected paragraph");
        };
        assert_eq!(
            text_of(&paragraph.children[0]),
            "https://docs.example/modbus#modbus"
        );
    }

    trait TagName {
        fn tag_name(&self) -> Option<&str>;
    }

    impl TagName for RenderableTreeNode {
        fn tag_name(&self) -> Option<&str> {
            match self {
                RenderableTreeNode::Tag(tag) => Some(tag.name.as_str()),
                RenderableTreeNode::Scalar(_) => None,
            }
        }
    }
}
