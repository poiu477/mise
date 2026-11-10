//! A small HTML document for reading recipe data: html5ever parses the page as a browser would, and this module
//! keeps what the reader needs (elements, their attributes, text) in an arena it can walk without recursion.
//!
//! html5ever's own example DOM (`markup5ever_rcdom`) is published for its tests and not meant for untrusted input,
//! so the tree is built here instead. The parser checks the elements it has open on every tag, so a page nested
//! thousands deep takes time growing with the square of its depth; reading stops once nesting passes
//! [`MAX_NESTING`], where browsers stop nesting too.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::{
    Attribute, ExpandedName, Parser, QualName, local_name, ns, parse_document, parse_fragment,
};
use serde_json::{Map, Value};

/// Nested microdata items deeper than this are left out; real recipes nest two or three levels.
const MAX_ITEM_DEPTH: usize = 16;

/// Elements nested deeper than this stop the reading. Chrome and Firefox stop nesting at 512 too; real pages stay
/// under a hundred.
pub(super) const MAX_NESTING: usize = 512;

/// How much of the page the parser takes at a time between checks of the nesting.
const CHUNK: usize = 16 * 1024;

/// Elements whose start and end break text into lines, so a list of steps reads as one step per line.
const BLOCKS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "br",
    "dd",
    "div",
    "dl",
    "dt",
    "figcaption",
    "figure",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "li",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "td",
    "th",
    "tr",
    "ul",
];

type NodeId = usize;

enum Data {
    Document,
    Element {
        name: Rc<QualName>,
        attrs: Vec<Attribute>,
        template: Option<NodeId>,
    },
    Text(String),
    Other,
}

struct Node {
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    /// How many elements deep the node was put, the document being 0.
    depth: usize,
    data: Data,
}

/// A parsed page or fragment. Node 0 is the document.
pub(super) struct Html {
    nodes: Vec<Node>,
}

impl Html {
    /// Parses a whole page. html5ever recovers from any markup, so the only failure is [`TooDeep`].
    pub(super) fn page(html: &str) -> Result<Html, TooDeep> {
        let (html, complete) = feed(parse_document(Builder::new(), Default::default()), html);
        if complete { Ok(html) } else { Err(TooDeep) }
    }

    /// Parses a piece of HTML found inside a value ("Stir in the <b>cheese</b> &amp; serve") as the inside of a
    /// `<div>`. Nesting past [`MAX_NESTING`] ends it early, keeping what came before.
    pub(super) fn fragment(html: &str) -> Html {
        let context = QualName::new(None, ns!(html), local_name!("div"));
        feed(
            parse_fragment(
                Builder::new(),
                Default::default(),
                context,
                Vec::new(),
                false,
            ),
            html,
        )
        .0
    }

    /// Every element in document order. The contents of `<template>` elements are left out, as a browser does.
    pub(super) fn elements(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.descendants(0)
            .into_iter()
            .filter(|&id| matches!(self.nodes[id].data, Data::Element { .. }))
    }

    /// The element's local name in lowercase ("script"), or "" for anything that isn't an element.
    pub(super) fn tag(&self, id: NodeId) -> &str {
        match &self.nodes[id].data {
            Data::Element { name, .. } => &name.local,
            _ => "",
        }
    }

    /// The value of an attribute, if the element has it.
    pub(super) fn attr(&self, id: NodeId, attr: &str) -> Option<&str> {
        match &self.nodes[id].data {
            Data::Element { attrs, .. } => attrs
                .iter()
                .find(|a| a.name.ns == ns!() && &*a.name.local == attr)
                .map(|a| &*a.value),
            _ => None,
        }
    }

    /// The text inside a node, with a line break around each block element ("li", "p", "br") so a list reads as
    /// lines. Whitespace inside a line is left as it is.
    pub(super) fn text(&self, id: NodeId) -> String {
        enum Visit {
            Open(NodeId),
            Close,
        }
        let mut out = String::new();
        let mut stack = vec![Visit::Open(id)];
        while let Some(visit) = stack.pop() {
            let id = match visit {
                Visit::Open(id) => id,
                Visit::Close => {
                    out.push('\n');
                    continue;
                }
            };
            match &self.nodes[id].data {
                Data::Text(text) => out.push_str(text),
                Data::Element { name, .. }
                    if matches!(&*name.local, "script" | "style" | "template") => {}
                Data::Element { name, .. } => {
                    if BLOCKS.contains(&&*name.local) {
                        out.push('\n');
                        stack.push(Visit::Close);
                    }
                    stack.extend(
                        self.nodes[id]
                            .children
                            .iter()
                            .rev()
                            .map(|&c| Visit::Open(c)),
                    );
                }
                Data::Document => {
                    stack.extend(
                        self.nodes[id]
                            .children
                            .iter()
                            .rev()
                            .map(|&c| Visit::Open(c)),
                    );
                }
                Data::Other => {}
            }
        }
        out
    }

    /// The whole document's text, as [`Html::text`] gives it.
    pub(super) fn all_text(&self) -> String {
        self.text(0)
    }

    /// Every JSON-LD block's contents, in page order: `<script type="application/ld+json">`.
    pub(super) fn json_ld_blocks(&self) -> Vec<String> {
        self.elements()
            .filter(|&id| self.tag(id) == "script")
            .filter(|&id| {
                self.attr(id, "type").is_some_and(|t| {
                    let media = t.split(';').next().unwrap_or("").trim();
                    media.eq_ignore_ascii_case("application/ld+json")
                })
            })
            .map(|id| {
                self.nodes[id]
                    .children
                    .iter()
                    .filter_map(|&c| match &self.nodes[c].data {
                        Data::Text(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect()
            })
            .collect()
    }

    /// The first `<link rel="…">` with this relation, or `<meta property="…">` / `<meta name="…">` with this
    /// name: its `href` or `content`.
    pub(super) fn head_value(&self, link_rel: Option<&str>, meta: Option<&str>) -> Option<String> {
        self.elements().find_map(|id| match self.tag(id) {
            "link" => {
                let rel = link_rel?;
                let rels = self.attr(id, "rel")?;
                rels.split_ascii_whitespace()
                    .any(|r| r.eq_ignore_ascii_case(rel))
                    .then(|| self.attr(id, "href").map(str::to_owned))
                    .flatten()
            }
            "meta" => {
                let meta = meta?;
                let key = self
                    .attr(id, "property")
                    .or_else(|| self.attr(id, "name"))?;
                key.eq_ignore_ascii_case(meta)
                    .then(|| self.attr(id, "content").map(str::to_owned))
                    .flatten()
            }
            _ => None,
        })
    }

    /// The `href` of the page's first `<base>`.
    pub(super) fn base_href(&self) -> Option<&str> {
        self.elements()
            .find(|&id| self.tag(id) == "base" && self.attr(id, "href").is_some())
            .and_then(|id| self.attr(id, "href"))
    }

    /// The text of the page's `<title>`.
    pub(super) fn title(&self) -> Option<String> {
        self.elements()
            .find(|&id| self.tag(id) == "title")
            .map(|id| self.text(id))
    }

    /// Every microdata item whose type is one of `types` ("Recipe"), in page order, as JSON shaped like JSON-LD
    /// (`{"@type": "Recipe", "name": "…", "recipeIngredient": ["…", "…"]}`) so one reader handles both.
    /// Properties named by `itemref` are not followed.
    pub(super) fn microdata_items(&self, types: &[&str]) -> Vec<Value> {
        self.elements()
            .filter(|&id| self.attr(id, "itemscope").is_some())
            .filter(|&id| {
                self.attr(id, "itemtype").is_some_and(|t| {
                    t.split_ascii_whitespace()
                        .any(|t| types.contains(&last_segment(t)))
                })
            })
            .map(|id| self.item(id, 0))
            .collect()
    }

    fn item(&self, id: NodeId, depth: usize) -> Value {
        let mut item = Map::new();
        if let Some(types) = self.attr(id, "itemtype") {
            let types: Vec<Value> = types
                .split_ascii_whitespace()
                .map(|t| Value::String(last_segment(t).to_owned()))
                .collect();
            match types.len() {
                0 => {}
                1 => {
                    item.insert("@type".into(), types[0].clone());
                }
                _ => {
                    item.insert("@type".into(), Value::Array(types));
                }
            }
        }
        // A property belongs to the nearest item around it, so the walk doesn't enter nested items.
        let mut stack: Vec<NodeId> = self.nodes[id].children.iter().rev().copied().collect();
        while let Some(el) = stack.pop() {
            if !matches!(self.nodes[el].data, Data::Element { .. }) {
                continue;
            }
            let nested = self.attr(el, "itemscope").is_some();
            if let Some(props) = self.attr(el, "itemprop") {
                let value = if nested {
                    (depth < MAX_ITEM_DEPTH).then(|| self.item(el, depth + 1))
                } else {
                    Some(Value::String(self.property_value(el)))
                };
                for prop in props.split_ascii_whitespace() {
                    if let Some(value) = &value {
                        add_property(&mut item, last_segment(prop), value.clone());
                    }
                }
            }
            if !nested {
                stack.extend(self.nodes[el].children.iter().rev());
            }
        }
        Value::Object(item)
    }

    /// A property's value as microdata defines it (`src` of an image, `href` of a link, `datetime` of a time), and
    /// the `content` attribute on any element, which recipe plugins use for machine-readable times.
    fn property_value(&self, id: NodeId) -> String {
        if let Some(content) = self.attr(id, "content") {
            return content.to_owned();
        }
        let from = match self.tag(id) {
            "audio" | "embed" | "iframe" | "img" | "source" | "track" | "video" => Some("src"),
            "a" | "area" | "link" => Some("href"),
            "object" => Some("data"),
            "data" | "meter" => Some("value"),
            "time" => Some("datetime"),
            _ => None,
        };
        from.and_then(|attr| self.attr(id, attr))
            .map(str::to_owned)
            .unwrap_or_else(|| self.text(id).trim().to_owned())
    }

    /// The node and everything inside it, in document order.
    fn descendants(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(id) = stack.pop() {
            out.push(id);
            stack.extend(self.nodes[id].children.iter().rev());
        }
        out
    }
}

/// A page nests elements deeper than [`MAX_NESTING`].
#[derive(Debug)]
pub(super) struct TooDeep;

/// Gives the parser the text a chunk at a time, stopping when nesting passes [`MAX_NESTING`]. Says whether it read
/// to the end.
fn feed(mut parser: Parser<Builder>, html: &str) -> (Html, bool) {
    let mut rest = html;
    while !rest.is_empty() {
        let mut at = rest.len().min(CHUNK);
        while !rest.is_char_boundary(at) {
            at += 1;
        }
        let (chunk, after) = rest.split_at(at);
        parser.process(StrTendril::from_slice(chunk));
        if parser.tokenizer.sink.sink.too_deep.get() {
            return (parser.finish(), false);
        }
        rest = after;
    }
    (parser.finish(), true)
}

/// "https://schema.org/Recipe" → "Recipe"; "schema:Recipe" → "Recipe"; "Recipe" stays.
pub(super) fn last_segment(name: &str) -> &str {
    name.rsplit(['/', '#', ':']).next().unwrap_or(name)
}

/// Adds a value to a property, turning it into a list when the property repeats (each ingredient is its own
/// `itemprop="recipeIngredient"`).
fn add_property(item: &mut Map<String, Value>, name: &str, value: Value) {
    match item.get_mut(name) {
        None => {
            item.insert(name.to_owned(), value);
        }
        Some(Value::Array(values)) => values.push(value),
        Some(existing) => {
            let first = existing.take();
            *existing = Value::Array(vec![first, value]);
        }
    }
}

/// Builds an [`Html`] for html5ever. The tree builder calls these as it reads; each call keeps the arena a tree.
struct Builder {
    nodes: RefCell<Vec<Node>>,
    too_deep: Cell<bool>,
}

/// A node as the tree builder holds it. The name rides along so `elem_name` can lend it out without a borrow of
/// the arena.
#[derive(Clone)]
struct Handle {
    id: NodeId,
    name: Option<Rc<QualName>>,
}

impl Builder {
    fn new() -> Builder {
        Builder {
            nodes: RefCell::new(vec![Node {
                parent: None,
                children: Vec::new(),
                depth: 0,
                data: Data::Document,
            }]),
            too_deep: Cell::new(false),
        }
    }

    fn add(&self, data: Data) -> NodeId {
        let mut nodes = self.nodes.borrow_mut();
        nodes.push(Node {
            parent: None,
            children: Vec::new(),
            depth: 0,
            data,
        });
        nodes.len() - 1
    }

    fn handle(id: NodeId) -> Handle {
        Handle { id, name: None }
    }

    /// Puts a node or text into `parent` at `index` (the end when `None`), joining text to a text node just
    /// before it.
    fn insert(&self, parent: NodeId, index: Option<usize>, child: NodeOrText<Handle>) {
        let mut nodes = self.nodes.borrow_mut();
        let index = index.unwrap_or(nodes[parent].children.len());
        let child = match child {
            NodeOrText::AppendText(text) => {
                let before = index.checked_sub(1).map(|i| nodes[parent].children[i]);
                if let Some(before) = before
                    && let Data::Text(existing) = &mut nodes[before].data
                {
                    existing.push_str(&text);
                    return;
                }
                nodes.push(Node {
                    parent: None,
                    children: Vec::new(),
                    depth: 0,
                    data: Data::Text(text.to_string()),
                });
                nodes.len() - 1
            }
            NodeOrText::AppendNode(handle) => handle.id,
        };
        // The parser adds to the element it has open, so this depth is how many elements it has open.
        let depth = nodes[parent].depth + 1;
        if depth > MAX_NESTING {
            self.too_deep.set(true);
        }
        nodes[child].depth = depth;
        nodes[child].parent = Some(parent);
        nodes[parent].children.insert(index, child);
    }

    fn detach(&self, id: NodeId) {
        let mut nodes = self.nodes.borrow_mut();
        if let Some(parent) = nodes[id].parent.take() {
            nodes[parent].children.retain(|&c| c != id);
        }
    }
}

impl TreeSink for Builder {
    type Handle = Handle;
    type Output = Html;
    type ElemName<'a> = ExpandedName<'a>;

    fn finish(self) -> Html {
        Html {
            nodes: self.nodes.into_inner(),
        }
    }

    fn parse_error(&self, _msg: Cow<'static, str>) {}

    fn get_document(&self) -> Handle {
        Builder::handle(0)
    }

    fn elem_name<'a>(&'a self, target: &'a Handle) -> ExpandedName<'a> {
        // The tree builder only asks for the names of elements it made with `create_element`.
        target
            .name
            .as_ref()
            .expect("the tree builder asks only for element names")
            .expanded()
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> Handle {
        let template = flags.template.then(|| self.add(Data::Document));
        let name = Rc::new(name);
        let id = self.add(Data::Element {
            name: name.clone(),
            attrs,
            template,
        });
        Handle {
            id,
            name: Some(name),
        }
    }

    fn create_comment(&self, _text: StrTendril) -> Handle {
        Builder::handle(self.add(Data::Other))
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> Handle {
        Builder::handle(self.add(Data::Other))
    }

    fn append(&self, parent: &Handle, child: NodeOrText<Handle>) {
        if let NodeOrText::AppendNode(node) = &child {
            self.detach(node.id);
        }
        self.insert(parent.id, None, child);
    }

    fn append_based_on_parent_node(
        &self,
        element: &Handle,
        prev_element: &Handle,
        child: NodeOrText<Handle>,
    ) {
        let has_parent = self.nodes.borrow()[element.id].parent.is_some();
        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(
        &self,
        _name: StrTendril,
        _public_id: StrTendril,
        _system_id: StrTendril,
    ) {
    }

    fn get_template_contents(&self, target: &Handle) -> Handle {
        let nodes = self.nodes.borrow();
        match &nodes[target.id].data {
            Data::Element {
                template: Some(contents),
                ..
            } => Builder::handle(*contents),
            // The tree builder asks only about templates; any other element has no separate contents.
            _ => target.clone(),
        }
    }

    fn same_node(&self, x: &Handle, y: &Handle) -> bool {
        x.id == y.id
    }

    fn set_quirks_mode(&self, _mode: QuirksMode) {}

    fn append_before_sibling(&self, sibling: &Handle, new_node: NodeOrText<Handle>) {
        if let NodeOrText::AppendNode(node) = &new_node {
            self.detach(node.id);
        }
        let place = {
            let nodes = self.nodes.borrow();
            nodes[sibling.id].parent.and_then(|parent| {
                let index = nodes[parent]
                    .children
                    .iter()
                    .position(|&c| c == sibling.id)?;
                Some((parent, index))
            })
        };
        if let Some((parent, index)) = place {
            self.insert(parent, Some(index), new_node);
        }
    }

    fn add_attrs_if_missing(&self, target: &Handle, attrs: Vec<Attribute>) {
        let mut nodes = self.nodes.borrow_mut();
        if let Data::Element {
            attrs: existing, ..
        } = &mut nodes[target.id].data
        {
            for attr in attrs {
                if !existing.iter().any(|a| a.name == attr.name) {
                    existing.push(attr);
                }
            }
        }
    }

    fn remove_from_parent(&self, target: &Handle) {
        self.detach(target.id);
    }

    fn reparent_children(&self, node: &Handle, new_parent: &Handle) {
        let mut nodes = self.nodes.borrow_mut();
        let children = std::mem::take(&mut nodes[node.id].children);
        let depth = nodes[new_parent.id].depth + 1;
        for &child in &children {
            nodes[child].parent = Some(new_parent.id);
            nodes[child].depth = depth;
        }
        nodes[new_parent.id].children.extend(children);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(html: &str) -> Html {
        Html::page(html).unwrap()
    }

    #[test]
    fn nesting_past_the_limit_stops_the_reading() {
        assert!(Html::page(&"<div>".repeat(MAX_NESTING - 3)).is_ok());
        assert!(Html::page(&"<div>".repeat(MAX_NESTING + 1)).is_err());
        // Formatting elements the parser opens again after each paragraph count too.
        let reopened: String = (0..MAX_NESTING * 2)
            .map(|i| format!("<p><b id={i}></p>"))
            .collect();
        assert!(Html::page(&reopened).is_err());
    }

    #[test]
    fn unclosed_list_items_end_where_a_browser_ends_them() {
        let html = page("<ul><li itemprop=a>1 cup rice<li itemprop=a>2 cups water</ul>");
        let items: Vec<String> = html
            .elements()
            .filter(|&id| html.tag(id) == "li")
            .map(|id| html.text(id).trim().to_owned())
            .collect();
        assert_eq!(items, ["1 cup rice", "2 cups water"]);
    }

    #[test]
    fn text_breaks_lines_at_blocks_and_decodes_entities() {
        let html = Html::fragment("<p>Salt &amp; pepper</p><p>Serve&nbsp;hot<br>Enjoy</p>");
        let text = html.all_text();
        let lines: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(lines, ["Salt & pepper", "Serve\u{a0}hot", "Enjoy"]);
    }

    #[test]
    fn json_ld_blocks_are_read_raw_and_others_skipped() {
        let html = page(
            r#"<script type="application/ld+json; charset=utf-8">{"a": "<b>&amp;</b>"}</script>
               <script type="text/javascript">var x = 1;</script>
               <template><script type="application/ld+json">{"hidden": true}</script></template>"#,
        );
        assert_eq!(html.json_ld_blocks(), [r#"{"a": "<b>&amp;</b>"}"#]);
    }

    #[test]
    fn misnested_tags_still_make_a_tree() {
        // The adoption agency algorithm moves nodes around; every node must end up with one parent.
        let html = page("<p><b>1<i>2</b>3</i>4<table><td>5</table><b>6");
        let text: String = html.all_text().split_whitespace().collect();
        assert_eq!(text, "123456");
        for (id, node) in html.nodes.iter().enumerate() {
            for &child in &node.children {
                assert_eq!(html.nodes[child].parent, Some(id));
            }
        }
    }

    #[test]
    fn microdata_properties_stay_with_their_own_item() {
        let html = page(
            r#"<div itemscope itemtype="https://schema.org/Recipe">
                 <h1 itemprop="name">Toast</h1>
                 <span itemprop="author" itemscope itemtype="https://schema.org/Person">
                   <span itemprop="name">Sam</span></span>
                 <meta itemprop="prepTime" content="PT5M">
                 <img itemprop="image" src="/toast.jpg">
                 <span itemprop="recipeIngredient">Bread</span>
                 <span itemprop="recipeIngredient">Butter</span>
               </div>"#,
        );
        let items = html.microdata_items(&["Recipe"]);
        assert_eq!(
            items,
            [serde_json::json!({
                "@type": "Recipe",
                "name": "Toast",
                "author": {"@type": "Person", "name": "Sam"},
                "prepTime": "PT5M",
                "image": "/toast.jpg",
                "recipeIngredient": ["Bread", "Butter"],
            })]
        );
    }
}
