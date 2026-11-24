//! Trees shaped like the pages the design's targets were measured on (design 4.1), built from a
//! seed. Only their shape is copied, never a page's text: the node count, the JSON's size, the
//! role mix, how deep the nodes sit, how long the names, texts and urls run. No real page goes
//! to the pool.
//!
//! | Shape | Measured page | Nodes | JSON |
//! |---|---|---:|---:|
//! | `thread` | a discussion thread of about 116,000 elements, the largest measured | 49,882 | 4.25 MB |
//! | `article` | an encyclopedia article of about 17,000 elements | 7,352 | 0.92 MB |
//! | `grid` | a data grid of about 2,300 elements | 601 | 0.04 MB |
//!
//! The delta's worst case (design 4.1's row, as piece 5 timed it) is a page of its own: every node
//! prints one line, and 500 links spread from the first line to the last are renamed, which
//! makes 1,000 changed lines.

use neo_snapshot::{AriaChild, AriaNode};

/// SplitMix64: a small seeded generator, so every run builds the same trees.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `low..=high`.
    fn between(&mut self, low: usize, high: usize) -> usize {
        let span = (high - low + 1) as u64;
        low + (self.next() % span) as usize
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    /// A length drawn from `buckets`, each `(percent, low, high)`, the percents summing to 100:
    /// how a page's string lengths spread, a long tail included.
    fn length(&mut self, buckets: &[(u64, usize, usize)]) -> usize {
        let mut roll = self.next() % 100;
        for &(percent, low, high) in buckets {
            if roll < percent {
                return self.between(low, high);
            }
            roll -= percent;
        }
        buckets.last().map_or(1, |&(_, low, _)| low)
    }
}

const WORDS: &[&str] = &[
    "the", "of", "and", "page", "snapshot", "render", "tree", "line", "number", "with", "from",
    "about", "would", "there", "their", "which", "people", "system", "memory", "faster", "rust",
    "engine", "what", "when", "into", "other", "because", "time", "first", "could", "after",
];

/// Builds nodes with fresh refs and filler text of chosen lengths.
struct Builder {
    rng: Rng,
    next_ref: u32,
}

impl Builder {
    fn new(seed: u64) -> Self {
        Self {
            rng: Rng(seed),
            next_ref: 0,
        }
    }

    fn between(&mut self, low: usize, high: usize) -> usize {
        self.rng.between(low, high)
    }

    /// A node with only its role and a fresh ref, as the page's walk gives most nodes.
    fn node(&mut self, role: &str) -> AriaNode {
        self.next_ref += 1;
        let mut node = bare(role);
        node.r#ref = Some(format!("e{}", self.next_ref));
        node
    }

    /// A node holding `children`.
    fn with(&mut self, role: &str, children: Vec<AriaChild>) -> AriaNode {
        let mut node = self.node(role);
        node.children = children;
        node
    }

    /// A link the page marks clickable: a name (or none) and a url, each of a length drawn from
    /// its range, and the pointer cursor.
    fn link(&mut self, name: Option<(usize, usize)>, url: (usize, usize)) -> AriaNode {
        let mut link = self.node("link");
        link.name = name.map(|(low, high)| self.words_in(low, high));
        let url = self.between(url.0, url.1);
        link.url = Some(self.url(url));
        link.cursor = Some("pointer".to_owned());
        link
    }

    fn words_in(&mut self, low: usize, high: usize) -> String {
        let length = self.between(low, high);
        self.words(length)
    }

    fn text_in(&mut self, low: usize, high: usize) -> AriaChild {
        AriaChild::Text(self.words_in(low, high))
    }

    /// Words joined by spaces, cut to `length` bytes (ASCII, so bytes are UTF-16 units too).
    fn words(&mut self, length: usize) -> String {
        let mut text = String::with_capacity(length + 12);
        while text.len() < length {
            if !text.is_empty() {
                text.push(' ');
            }
            let word = WORDS[self.rng.between(0, WORDS.len() - 1)];
            text.push_str(word);
        }
        text.truncate(length);
        text
    }

    fn url(&mut self, length: usize) -> String {
        let mut url = String::from("/item?id=");
        while url.len() < length {
            url.push(char::from(b'0' + (self.rng.next() % 10) as u8));
        }
        url.truncate(length.max(1));
        url
    }

    fn text(&mut self, length: usize) -> AriaChild {
        AriaChild::Text(self.words(length))
    }
}

/// A node with every optional field left out.
fn bare(role: &str) -> AriaNode {
    AriaNode {
        role: role.to_owned(),
        name: None,
        checked: None,
        disabled: None,
        expanded: None,
        active: None,
        inert: None,
        on_top: None,
        invalid: None,
        level: None,
        pressed: None,
        selected: None,
        aria_hidden: None,
        r#ref: None,
        cursor: None,
        r#box: None,
        url: None,
        placeholder: None,
        text: None,
        children: Vec::new(),
    }
}

fn child(node: AriaNode) -> AriaChild {
    AriaChild::Node(Box::new(node))
}

/// The largest measured page: one table of comments, each a row holding a table of its own with
/// a vote link, a header of links, its text and a reply link. Seven to eight links and 22 to 23
/// nodes a comment; most strings short, a few long.
pub fn thread() -> Vec<AriaNode> {
    const COMMENTS: usize = 2_236;
    let mut b = Builder::new(0x7468_7265_6164);
    let mut rows = Vec::with_capacity(COMMENTS + 1);
    let mut header = Vec::new();
    for _ in 0..8 {
        header.push(child(b.link(Some((3, 9)), (12, 12))));
        header.push(AriaChild::Text(" | ".to_owned()));
    }
    let header = b.with("generic", header);
    let header = b.with("cell", vec![child(header)]);
    rows.push(child(b.with("row", vec![child(header)])));
    for _ in 0..COMMENTS {
        rows.push(child(comment(&mut b)));
    }
    let rowgroup = b.with("rowgroup", rows);
    vec![b.with("table", vec![child(rowgroup)])]
}

fn comment(b: &mut Builder) -> AriaNode {
    let mut indent = b.node("cell");
    indent.aria_hidden = Some(true);
    let vote = b.link(None, (58, 58));
    let vote = b.with("cell", vec![child(vote)]);

    let user = b.link(Some((3, 15)), (16, 16));
    let age = b.link(Some((9, 14)), (16, 16));
    let age = b.with("generic", vec![child(age)]);
    let mut hide = b.link(None, (9, 9));
    hide.text = Some("[-]".to_owned());
    hide.aria_hidden = Some(true);
    let parent = b.link(Some((6, 6)), (18, 18));
    let next = b.link(Some((4, 4)), (18, 18));
    let navigation = b.with(
        "generic",
        vec![
            AriaChild::Text(" | ".to_owned()),
            child(hide),
            AriaChild::Text(" | ".to_owned()),
            child(parent),
            AriaChild::Text(" | ".to_owned()),
            child(next),
        ],
    );
    let head = b.with("generic", vec![child(user), child(age), child(navigation)]);

    let opening = b
        .rng
        .length(&[(25, 1, 40), (40, 60, 260), (28, 260, 500), (7, 500, 1300)]);
    let mut said = vec![b.text(opening)];
    for _ in 0..b.rng.between(0, 2) {
        let length = b.rng.length(&[(40, 4, 60), (45, 60, 240), (15, 240, 900)]);
        let mut paragraph = b.node("paragraph");
        if b.rng.chance(25) {
            let address = b.rng.between(30, 110);
            let named = b.link(Some((address.min(70), address.min(70))), (address, address));
            paragraph.children = vec![b.text(length / 2), child(named), b.text(length / 2)];
        } else {
            paragraph.text = Some(b.words(length));
        }
        said.push(child(paragraph));
    }
    let said = b.with("generic", said);
    let reply = b.link(Some((5, 5)), (54, 54));
    let reply = b.with("paragraph", vec![child(reply)]);
    let body = b.with("generic", vec![child(said), child(reply)]);
    let content = b.with("cell", vec![child(head), child(body)]);

    let inner = b.with("row", vec![child(indent), child(vote), child(content)]);
    let inner = b.with("rowgroup", vec![child(inner)]);
    let inner = b.with("table", vec![child(inner)]);
    let cell = b.with("cell", vec![child(inner)]);
    b.with("row", vec![child(cell)])
}

/// The article: a banner and navigation lists, then sections of paragraphs where text runs
/// between links and footnote links, a few figures and tables, and a long reference list.
pub fn article() -> Vec<AriaNode> {
    let mut b = Builder::new(0x6172_7469_636C);
    let mut top = Vec::new();

    let search_box = {
        let mut field = b.node("searchbox");
        field.name = Some("Search".to_owned());
        field
    };
    let mut go = b.node("button");
    go.name = Some("Search".to_owned());
    let search = b.with("search", vec![child(search_box), child(go)]);
    let logo = b.link(Some((9, 9)), (20, 20));
    let banner = b.with("banner", vec![child(logo), child(search)]);
    top.push(child(banner));
    for links in [12, 60] {
        let items = (0..links)
            .map(|_| {
                let link = b.link(Some((5, 24)), (20, 50));
                child(b.with("listitem", vec![child(link)]))
            })
            .collect();
        let list = b.with("list", items);
        top.push(child(b.with("navigation", vec![child(list)])));
    }

    let mut main = Vec::new();
    let mut title = b.node("heading");
    title.name = Some(b.words(14));
    title.level = Some(1.into());
    main.push(child(title));
    for _ in 0..44 {
        let mut section = Vec::new();
        let mut heading = b.node("heading");
        heading.name = Some(b.words_in(6, 30));
        heading.level = Some(2.into());
        section.push(child(heading));
        for _ in 0..b.rng.between(2, 5) {
            section.push(child(paragraph(&mut b)));
        }
        if b.rng.chance(40) {
            let mut image = b.node("img");
            image.name = Some(b.words_in(10, 60));
            let caption = b.text_in(40, 160);
            section.push(child(b.with("figure", vec![child(image), caption])));
        }
        if b.rng.chance(20) {
            section.push(child(table(&mut b)));
        }
        main.push(child(b.with("region", section)));
    }
    let references = (0..975)
        .map(|_| {
            let mut back = b.link(Some((7, 7)), (38, 38));
            back.text = Some("^".to_owned());
            let source = b.link(Some((8, 20)), (15, 60));
            let length = b.rng.length(&[(70, 1, 20), (25, 20, 120), (5, 120, 400)]);
            let note = b.text(length);
            let cited = b.with("generic", vec![child(source), note]);
            child(b.with("listitem", vec![child(back), child(cited)]))
        })
        .collect();
    main.push(child(b.with("list", references)));
    for _ in 0..90 {
        let mut long = b.node("listitem");
        long.text = Some(b.words_in(200, 680));
        main.push(child(long));
    }
    top.push(child(b.with("main", main)));

    let footer = (0..10)
        .map(|_| {
            let link = b.link(Some((6, 20)), (30, 30));
            child(b.with("listitem", vec![child(link)]))
        })
        .collect();
    let footer = b.with("list", footer);
    top.push(child(b.with("contentinfo", vec![child(footer)])));
    vec![b.with("generic", top)]
}

/// One paragraph of the article: text pieces between links, some links as footnotes.
fn paragraph(b: &mut Builder) -> AriaNode {
    let mut pieces = Vec::new();
    for _ in 0..b.rng.between(8, 18) {
        let length = b.rng.length(&[(25, 1, 15), (50, 15, 90), (25, 90, 220)]);
        pieces.push(b.text(length));
        if b.rng.chance(70) {
            pieces.push(child(b.link(Some((5, 40)), (40, 100))));
        } else {
            let note = b.link(Some((4, 4)), (13, 52));
            pieces.push(child(b.with("superscript", vec![child(note)])));
        }
    }
    b.with("paragraph", pieces)
}

fn table(b: &mut Builder) -> AriaNode {
    let rows = (0..6)
        .map(|_| {
            let cells = (0..3)
                .map(|_| {
                    let mut cell = b.node("cell");
                    cell.name = Some(b.words_in(3, 20));
                    child(cell)
                })
                .collect();
            child(b.with("row", cells))
        })
        .collect();
    let rowgroup = b.with("rowgroup", rows);
    b.with("table", vec![child(rowgroup)])
}

/// The grid: tabs, a toolbar of fields, a tree, a list box, and a grid whose rows hold a row
/// header, check boxes, named cells and cells of icons.
pub fn grid() -> Vec<AriaNode> {
    let mut b = Builder::new(0x6772_6964);
    let mut panel = Vec::new();

    let tabs = (0..2)
        .map(|_| {
            let mut tab = b.node("tab");
            tab.name = Some(b.words_in(4, 12));
            tab.cursor = Some("pointer".to_owned());
            child(tab)
        })
        .collect();
    let tablist = b.with("tablist", tabs);

    for _ in 0..8 {
        let mut field = b.node("textbox");
        field.name = Some(b.words_in(4, 16));
        panel.push(child(field));
    }
    for _ in 0..15 {
        let mut button = b.node("button");
        button.name = Some(b.words_in(3, 14));
        panel.push(child(button));
    }
    let items = (0..9)
        .map(|level| {
            let mut item = b.node("treeitem");
            item.name = Some(b.words_in(5, 18));
            item.level = Some((level % 3 + 1).into());
            child(item)
        })
        .collect();
    panel.push(child(b.with("tree", items)));
    let options = (0..16)
        .map(|_| {
            let mut option = b.node("option");
            option.name = Some(b.words_in(4, 20));
            child(option)
        })
        .collect();
    panel.push(child(b.with("listbox", options)));

    let header = (0..15)
        .map(|_| {
            let mut column = b.node("columnheader");
            column.name = Some(b.words_in(4, 14));
            child(column)
        })
        .collect();
    let mut rows = vec![child(b.with("row", header))];
    for _ in 0..25 {
        rows.push(child(grid_row(&mut b)));
    }
    let rowgroup = b.with("rowgroup", rows);
    panel.push(child(b.with("grid", vec![child(rowgroup)])));
    for _ in 0..22 {
        panel.push(child(b.link(Some((6, 30)), (15, 73))));
    }
    let tabpanel = b.with("tabpanel", panel);
    vec![b.with("main", vec![child(tablist), child(tabpanel)])]
}

fn grid_row(b: &mut Builder) -> AriaNode {
    let mut cells = Vec::new();
    let mut number = b.node("rowheader");
    number.name = Some(b.words(1));
    cells.push(child(number));
    for column in 0..10 {
        let cell = match column {
            0 | 5 => {
                let mut check = b.node("checkbox");
                check.name = Some(b.words_in(40, 48));
                check.cursor = Some("pointer".to_owned());
                if b.rng.chance(40) {
                    check.checked = Some(neo_snapshot::Tristate::True);
                }
                b.with("gridcell", vec![child(check)])
            }
            3 | 7 => {
                let icons = (0..b.rng.between(1, 3))
                    .map(|_| {
                        let mut icon = b.node("img");
                        icon.name = Some(b.words_in(6, 13));
                        child(icon)
                    })
                    .collect();
                let icons = b.with("generic", icons);
                b.with("gridcell", vec![child(icons)])
            }
            _ => {
                let mut cell = b.node("gridcell");
                cell.name = Some(b.words_in(3, 20));
                cell
            }
        };
        cells.push(child(cell));
    }
    b.with("row", cells)
}

/// The delta's worst case, as a page and its changed copy. `repeated` false: every line distinct,
/// one link a line. `repeated` true: every line that does not change is one of two headings, so
/// the search meets the most ties; the 500 links stand among them.
pub fn delta_pair(repeated: bool) -> (Vec<AriaNode>, Vec<AriaNode>) {
    const LINES: usize = 33_301;
    const RENAMED: usize = 500;
    let mut b = Builder::new(if repeated {
        0x7265_7065_6174
    } else {
        0x6469_7374
    });
    let renamed: Vec<usize> = (0..RENAMED)
        .map(|n| n * (LINES - 1) / (RENAMED - 1))
        .collect();
    let mut before = Vec::with_capacity(LINES);
    let mut after = Vec::with_capacity(LINES);
    let mut next = renamed.iter().peekable();
    for line in 0..LINES {
        let is_renamed = next.next_if_eq(&&line).is_some();
        let node = if repeated && !is_renamed {
            let mut heading = bare("heading");
            heading.name = Some(if line % 2 == 0 { "even" } else { "odd" }.to_owned());
            heading
        } else {
            let mut link = b.node("link");
            link.name = Some(format!("item {line}"));
            link.cursor = Some("pointer".to_owned());
            link
        };
        let mut changed = node.clone();
        if is_renamed && let Some(name) = &mut changed.name {
            name.push_str(" renamed");
        }
        before.push(child(node));
        after.push(child(changed));
    }
    let mut page = bare("generic");
    page.children = before;
    let mut changed = bare("generic");
    changed.children = after;
    (vec![page], vec![changed])
}

/// The tree's node count, walked with a stack of its own.
pub fn nodes(tree: &[AriaNode]) -> usize {
    let mut count = 0;
    let mut pending: Vec<&AriaNode> = tree.iter().collect();
    while let Some(node) = pending.pop() {
        count += 1;
        for entry in &node.children {
            if let AriaChild::Node(inner) = entry {
                pending.push(inner);
            }
        }
    }
    count
}
