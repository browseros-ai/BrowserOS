use prost::Enumeration;

/// Chromium accessibility role carried on [`ContentAttributes::aria_role`](super::ContentAttributes::aria_role).
///
/// Mirrors all 214 variants (`0..=213`) of `optimization_guide.proto.AXRole` from
/// `common_quality_data.proto`, including `MenuItemSeparator = 213` which appears out of numeric
/// order in the upstream proto.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Enumeration)]
#[repr(i32)]
pub enum AxRole {
    /// `AX_ROLE_NONE`: no accessibility role.
    None = 0,
    /// `AX_ROLE_ABBR`: abbreviation (`<abbr>`).
    Abbr = 1,
    /// `AX_ROLE_ALERT`: live alert message (`role="alert"`).
    Alert = 2,
    /// `AX_ROLE_ALERTDIALOG`: alert dialog (`role="alertdialog"`).
    AlertDialog = 3,
    /// `AX_ROLE_APPLICATION`: interactive application region (`role="application"`).
    Application = 4,
    /// `AX_ROLE_ARTICLE`: standalone article (`<article>` or `role="article"`).
    Article = 5,
    /// `AX_ROLE_AUDIO`: audio media element (`<audio>`).
    Audio = 6,
    /// `AX_ROLE_BANNER`: site banner landmark (`role="banner"`).
    Banner = 7,
    /// `AX_ROLE_BLOCKQUOTE`: block quotation (`<blockquote>`).
    Blockquote = 8,
    /// `AX_ROLE_BUTTON`: clickable push button (`<button>` or `role="button"`).
    Button = 9,
    /// `AX_ROLE_CANVAS`: drawing canvas (`<canvas>`).
    Canvas = 10,
    /// `AX_ROLE_CAPTION`: table or figure caption (`<caption>`).
    Caption = 11,
    /// `AX_ROLE_CARET`: text insertion caret.
    Caret = 12,
    /// `AX_ROLE_CELL`: table data cell (`<td>` or `role="cell"`).
    Cell = 13,
    /// `AX_ROLE_CHECKBOX`: checkable box (`<input type="checkbox">` or `role="checkbox"`).
    CheckBox = 14,
    /// `AX_ROLE_CLIENT`: desktop client window area.
    Client = 15,
    /// `AX_ROLE_CODE`: inline code (`<code>` or `role="code"`).
    Code = 16,
    /// `AX_ROLE_COLORWELL`: color picker control (`<input type="color">`).
    ColorWell = 17,
    /// `AX_ROLE_COLUMN`: table column.
    Column = 18,
    /// `AX_ROLE_COLUMNHEADER`: table column header (`<th scope="col">` or `role="columnheader"`).
    ColumnHeader = 19,
    /// `AX_ROLE_COMBOBOXGROUPING`: composite combobox grouping (`role="combobox"`).
    ComboBoxGrouping = 20,
    /// `AX_ROLE_COMBOBOXMENUBUTTON`: combobox menu button control.
    ComboBoxMenuButton = 21,
    /// `AX_ROLE_COMPLEMENTARY`: complementary landmark (`<aside>` or `role="complementary"`).
    Complementary = 22,
    /// `AX_ROLE_COMMENT`: comment annotation (`role="comment"`).
    Comment = 23,
    /// `AX_ROLE_CONTENTDELETION`: deleted content (`<del>` or `role="deletion"`).
    ContentDeletion = 24,
    /// `AX_ROLE_CONTENTINSERTION`: inserted content (`<ins>` or `role="insertion"`).
    ContentInsertion = 25,
    /// `AX_ROLE_CONTENTINFO`: page content-info landmark (`role="contentinfo"`).
    ContentInfo = 26,
    /// `AX_ROLE_DATE`: date input control (`<input type="date">`).
    Date = 27,
    /// `AX_ROLE_DATETIME`: date-time input control (`<input type="datetime-local">`).
    DateTime = 28,
    /// `AX_ROLE_DEFINITION`: term definition (`<dd>` or `role="definition"`).
    Definition = 29,
    /// `AX_ROLE_DESCRIPTIONLIST`: description list (`<dl>`).
    DescriptionList = 30,
    /// `AX_ROLE_DESCRIPTIONLISTDETAILDEPRECATED`: deprecated description list detail.
    DescriptionListDetailDeprecated = 31,
    /// `AX_ROLE_DESCRIPTIONLISTTERMDEPRECATED`: deprecated description list term.
    DescriptionListTermDeprecated = 32,
    /// `AX_ROLE_DESKTOP`: desktop root surface.
    Desktop = 33,
    /// `AX_ROLE_DETAILS`: disclosure details container (`<details>`).
    Details = 34,
    /// `AX_ROLE_DIALOG`: dialog container (`<dialog>` or `role="dialog"`).
    Dialog = 35,
    /// `AX_ROLE_DIRECTORYDEPRECATED`: deprecated directory list (`role="directory"`).
    DirectoryDeprecated = 36,
    /// `AX_ROLE_DISCLOSURETRIANGLE`: disclosure summary toggle (`<summary>`).
    DisclosureTriangle = 37,
    /// `AX_ROLE_DOCABSTRACT`: `DPUB` abstract role (`role="doc-abstract"`).
    DocAbstract = 38,
    /// `AX_ROLE_DOCACKNOWLEDGMENTS`: `DPUB` acknowledgments (`role="doc-acknowledgments"`).
    DocAcknowledgments = 39,
    /// `AX_ROLE_DOCAFTERWORD`: `DPUB` afterword (`role="doc-afterword"`).
    DocAfterword = 40,
    /// `AX_ROLE_DOCAPPENDIX`: `DPUB` appendix (`role="doc-appendix"`).
    DocAppendix = 41,
    /// `AX_ROLE_DOCBACKLINK`: `DPUB` backlink (`role="doc-backlink"`).
    DocBackLink = 42,
    /// `AX_ROLE_DOCBIBLIOENTRY`: `DPUB` bibliography entry (`role="doc-biblioentry"`).
    DocBiblioEntry = 43,
    /// `AX_ROLE_DOCBIBLIOGRAPHY`: `DPUB` bibliography (`role="doc-bibliography"`).
    DocBibliography = 44,
    /// `AX_ROLE_DOCBIBLIOREF`: `DPUB` bibliography reference (`role="doc-biblioref"`).
    DocBiblioRef = 45,
    /// `AX_ROLE_DOCCHAPTER`: `DPUB` chapter (`role="doc-chapter"`).
    DocChapter = 46,
    /// `AX_ROLE_DOCCOLOPHON`: `DPUB` colophon (`role="doc-colophon"`).
    DocColophon = 47,
    /// `AX_ROLE_DOCCONCLUSION`: `DPUB` conclusion (`role="doc-conclusion"`).
    DocConclusion = 48,
    /// `AX_ROLE_DOCCOVER`: `DPUB` cover (`role="doc-cover"`).
    DocCover = 49,
    /// `AX_ROLE_DOCCREDIT`: `DPUB` credit (`role="doc-credit"`).
    DocCredit = 50,
    /// `AX_ROLE_DOCCREDITS`: `DPUB` credits (`role="doc-credits"`).
    DocCredits = 51,
    /// `AX_ROLE_DOCDEDICATION`: `DPUB` dedication (`role="doc-dedication"`).
    DocDedication = 52,
    /// `AX_ROLE_DOCENDNOTE`: `DPUB` endnote (`role="doc-endnote"`).
    DocEndnote = 53,
    /// `AX_ROLE_DOCENDNOTES`: `DPUB` endnotes (`role="doc-endnotes"`).
    DocEndnotes = 54,
    /// `AX_ROLE_DOCEPIGRAPH`: `DPUB` epigraph (`role="doc-epigraph"`).
    DocEpigraph = 55,
    /// `AX_ROLE_DOCEPILOGUE`: `DPUB` epilogue (`role="doc-epilogue"`).
    DocEpilogue = 56,
    /// `AX_ROLE_DOCERRATA`: `DPUB` errata (`role="doc-errata"`).
    DocErrata = 57,
    /// `AX_ROLE_DOCEXAMPLE`: `DPUB` example (`role="doc-example"`).
    DocExample = 58,
    /// `AX_ROLE_DOCFOOTNOTE`: `DPUB` footnote (`role="doc-footnote"`).
    DocFootnote = 59,
    /// `AX_ROLE_DOCFOREWORD`: `DPUB` foreword (`role="doc-foreword"`).
    DocForeword = 60,
    /// `AX_ROLE_DOCGLOSSARY`: `DPUB` glossary (`role="doc-glossary"`).
    DocGlossary = 61,
    /// `AX_ROLE_DOCGLOSSREF`: `DPUB` glossary reference (`role="doc-glossref"`).
    DocGlossRef = 62,
    /// `AX_ROLE_DOCINDEX`: `DPUB` index (`role="doc-index"`).
    DocIndex = 63,
    /// `AX_ROLE_DOCINTRODUCTION`: `DPUB` introduction (`role="doc-introduction"`).
    DocIntroduction = 64,
    /// `AX_ROLE_DOCNOTEREF`: `DPUB` note reference (`role="doc-noteref"`).
    DocNoteRef = 65,
    /// `AX_ROLE_DOCNOTICE`: `DPUB` notice (`role="doc-notice"`).
    DocNotice = 66,
    /// `AX_ROLE_DOCPAGEBREAK`: `DPUB` page break (`role="doc-pagebreak"`).
    DocPageBreak = 67,
    /// `AX_ROLE_DOCPAGEFOOTER`: `DPUB` page footer (`role="doc-pagefooter"`).
    DocPageFooter = 68,
    /// `AX_ROLE_DOCPAGEHEADER`: `DPUB` page header (`role="doc-pageheader"`).
    DocPageHeader = 69,
    /// `AX_ROLE_DOCPAGELIST`: `DPUB` page list (`role="doc-pagelist"`).
    DocPageList = 70,
    /// `AX_ROLE_DOCPART`: `DPUB` part (`role="doc-part"`).
    DocPart = 71,
    /// `AX_ROLE_DOCPREFACE`: `DPUB` preface (`role="doc-preface"`).
    DocPreface = 72,
    /// `AX_ROLE_DOCPROLOGUE`: `DPUB` prologue (`role="doc-prologue"`).
    DocPrologue = 73,
    /// `AX_ROLE_DOCPULLQUOTE`: `DPUB` pull quote (`role="doc-pullquote"`).
    DocPullquote = 74,
    /// `AX_ROLE_DOCQNA`: `DPUB` questions and answers (`role="doc-qna"`).
    DocQna = 75,
    /// `AX_ROLE_DOCSUBTITLE`: `DPUB` subtitle (`role="doc-subtitle"`).
    DocSubtitle = 76,
    /// `AX_ROLE_DOCTIP`: `DPUB` tip (`role="doc-tip"`).
    DocTip = 77,
    /// `AX_ROLE_DOCTOC`: `DPUB` table of contents (`role="doc-toc"`).
    DocToc = 78,
    /// `AX_ROLE_DOCUMENT`: document content region (`role="document"`).
    Document = 79,
    /// `AX_ROLE_EMBEDDEDOBJECT`: embedded external object (`<embed>` or `<object>`).
    EmbeddedObject = 80,
    /// `AX_ROLE_EMPHASIS`: inline emphasis (`<em>` or `role="emphasis"`).
    Emphasis = 81,
    /// `AX_ROLE_FEED`: scrollable article feed (`role="feed"`).
    Feed = 82,
    /// `AX_ROLE_FIGCAPTION`: figure caption (`<figcaption>`).
    Figcaption = 83,
    /// `AX_ROLE_FIGURE`: figure container (`<figure>` or `role="figure"`).
    Figure = 84,
    /// `AX_ROLE_FOOTER`: footer container (`<footer>`).
    Footer = 85,
    /// `AX_ROLE_SECTIONFOOTER`: section-scoped footer (`<footer>` inside a sectioning element).
    SectionFooter = 86,
    /// `AX_ROLE_FORM`: form landmark (`<form>` or `role="form"`).
    Form = 87,
    /// `AX_ROLE_GENERICCONTAINER`: generic container (`<div>`, `<span>`, or `role="generic"`).
    GenericContainer = 88,
    /// `AX_ROLE_GRAPHICSDOCUMENT`: graphics document (`role="graphics-document"`).
    GraphicsDocument = 89,
    /// `AX_ROLE_GRAPHICSOBJECT`: graphics object (`role="graphics-object"`).
    GraphicsObject = 90,
    /// `AX_ROLE_GRAPHICSSYMBOL`: graphics symbol (`role="graphics-symbol"`).
    GraphicsSymbol = 91,
    /// `AX_ROLE_GRID`: interactive data grid (`role="grid"`).
    Grid = 92,
    /// `AX_ROLE_GROUP`: logical group of related elements (`role="group"`).
    Group = 93,
    /// `AX_ROLE_HEADER`: page header (`<header>`).
    Header = 94,
    /// `AX_ROLE_SECTIONHEADER`: section-scoped header (`<header>` inside a sectioning element).
    SectionHeader = 95,
    /// `AX_ROLE_HEADING`: heading (`<h1>`..`<h6>` or `role="heading"`).
    Heading = 96,
    /// `AX_ROLE_IFRAME`: embedded iframe (`<iframe>`).
    Iframe = 97,
    /// `AX_ROLE_IFRAMEPRESENTATIONAL`: presentational iframe (`<iframe role="presentation">`).
    IframePresentational = 98,
    /// `AX_ROLE_IMAGE`: image (`<img>` or `role="img"`).
    Image = 99,
    /// `AX_ROLE_IMECANDIDATE`: input method editor candidate window.
    ImeCandidate = 100,
    /// `AX_ROLE_INLINETEXTBOX`: line-layout inline text box.
    InlineTextBox = 101,
    /// `AX_ROLE_INPUTTIME`: time input control (`<input type="time">`).
    InputTime = 102,
    /// `AX_ROLE_KEYBOARD`: virtual keyboard surface.
    Keyboard = 103,
    /// `AX_ROLE_LABELTEXT`: form control label (`<label>`).
    LabelText = 104,
    /// `AX_ROLE_LAYOUTTABLE`: layout-only HTML table.
    LayoutTable = 105,
    /// `AX_ROLE_LAYOUTTABLECELL`: cell inside a layout-only table.
    LayoutTableCell = 106,
    /// `AX_ROLE_LAYOUTTABLEROW`: row inside a layout-only table.
    LayoutTableRow = 107,
    /// `AX_ROLE_LEGEND`: fieldset legend (`<legend>`).
    Legend = 108,
    /// `AX_ROLE_LINEBREAK`: explicit line break (`<br>`).
    LineBreak = 109,
    /// `AX_ROLE_LINK`: hyperlink (`<a href>` or `role="link"`).
    Link = 110,
    /// `AX_ROLE_LIST`: list container (`<ul>`, `<ol>`, or `role="list"`).
    List = 111,
    /// `AX_ROLE_LISTBOX`: listbox selector (`<select multiple>` or `role="listbox"`).
    ListBox = 112,
    /// `AX_ROLE_LISTBOXOPTION`: option inside a listbox (`role="option"`).
    ListBoxOption = 113,
    /// `AX_ROLE_LISTGRID`: list-style grid container.
    ListGrid = 114,
    /// `AX_ROLE_LISTITEM`: list item (`<li>` or `role="listitem"`).
    ListItem = 115,
    /// `AX_ROLE_LISTMARKER`: bullet or number marker on a list item.
    ListMarker = 116,
    /// `AX_ROLE_LOG`: live log region (`role="log"`).
    Log = 117,
    /// `AX_ROLE_MAIN`: main content landmark (`<main>` or `role="main"`).
    Main = 118,
    /// `AX_ROLE_MARK`: highlighted text (`<mark>` or `role="mark"`).
    Mark = 119,
    /// `AX_ROLE_MARQUEE`: marquee live region (`role="marquee"`).
    Marquee = 120,
    /// `AX_ROLE_MATH`: mathematical expression (`role="math"`).
    Math = 121,
    /// `AX_ROLE_MENU`: popup or context menu (`role="menu"`).
    Menu = 122,
    /// `AX_ROLE_MENUBAR`: persistent menu bar (`role="menubar"`).
    MenuBar = 123,
    /// `AX_ROLE_MENUITEM`: menu item (`role="menuitem"`).
    MenuItem = 124,
    /// `AX_ROLE_MENUITEMCHECKBOX`: checkable menu item (`role="menuitemcheckbox"`).
    MenuItemCheckBox = 125,
    /// `AX_ROLE_MENUITEMRADIO`: radio menu item (`role="menuitemradio"`).
    MenuItemRadio = 126,
    /// `AX_ROLE_MENUITEMSEPARATOR`: separator inside a menu (tag 213; out of numeric order in
    /// Chromium's `common_quality_data.proto`).
    MenuItemSeparator = 213,
    /// `AX_ROLE_MENULISTOPTION`: option inside a `<select>` popup menu.
    MenuListOption = 127,
    /// `AX_ROLE_MENULISTPOPUP`: popup menu owned by a `<select>` element.
    MenuListPopup = 128,
    /// `AX_ROLE_METER`: scalar meter (`<meter>` or `role="meter"`).
    Meter = 129,
    /// `AX_ROLE_NAVIGATION`: navigation landmark (`<nav>` or `role="navigation"`).
    Navigation = 130,
    /// `AX_ROLE_NOTE`: parenthetical note (`role="note"`).
    Note = 131,
    /// `AX_ROLE_PANE`: UI pane container.
    Pane = 132,
    /// `AX_ROLE_PARAGRAPH`: paragraph block (`<p>` or `role="paragraph"`).
    Paragraph = 133,
    /// `AX_ROLE_PDFACTIONABLEHIGHLIGHT`: actionable highlight annotation in a PDF document.
    PdfActionableHighlight = 134,
    /// `AX_ROLE_PDFROOT`: root accessibility node of a PDF viewer.
    PdfRoot = 135,
    /// `AX_ROLE_PLUGINOBJECT`: plugin object container.
    PluginObject = 136,
    /// `AX_ROLE_POPUPBUTTON`: button that opens a popup menu (such as collapsed `<select>`).
    PopUpButton = 137,
    /// `AX_ROLE_PORTALDEPRECATED`: deprecated HTML `<portal>` element.
    PortalDeprecated = 138,
    /// `AX_ROLE_PREDEPRECATED`: deprecated preformatted block role.
    PreDeprecated = 139,
    /// `AX_ROLE_PROGRESSINDICATOR`: progress indicator (`<progress>` or `role="progressbar"`).
    ProgressIndicator = 140,
    /// `AX_ROLE_RADIOBUTTON`: radio button (`<input type="radio">` or `role="radio"`).
    RadioButton = 141,
    /// `AX_ROLE_RADIOGROUP`: group of radio buttons (`role="radiogroup"`).
    RadioGroup = 142,
    /// `AX_ROLE_REGION`: named landmark region (`<section>` with accessible name or
    /// `role="region"`).
    Region = 143,
    /// `AX_ROLE_ROOTWEBAREA`: root web document area.
    RootWebArea = 144,
    /// `AX_ROLE_ROW`: table or grid row (`<tr>` or `role="row"`).
    Row = 145,
    /// `AX_ROLE_ROWGROUP`: table row group (`<thead>`, `<tbody>`, `<tfoot>`, or `role="rowgroup"`).
    RowGroup = 146,
    /// `AX_ROLE_ROWHEADER`: table row header (`<th scope="row">` or `role="rowheader"`).
    RowHeader = 147,
    /// `AX_ROLE_RUBY`: ruby annotation container (`<ruby>`).
    Ruby = 148,
    /// `AX_ROLE_RUBYANNOTATION`: ruby annotation text (`<rt>`).
    RubyAnnotation = 149,
    /// `AX_ROLE_SCROLLBAR`: scrollbar control (`role="scrollbar"`).
    ScrollBar = 150,
    /// `AX_ROLE_SCROLLVIEW`: native scroll view container.
    ScrollView = 151,
    /// `AX_ROLE_SEARCH`: search landmark (`<search>` or `role="search"`).
    Search = 152,
    /// `AX_ROLE_SEARCHBOX`: search input box (`<input type="search">` or `role="searchbox"`).
    SearchBox = 153,
    /// `AX_ROLE_SECTION`: generic document section.
    Section = 154,
    /// `AX_ROLE_SLIDER`: range slider (`<input type="range">` or `role="slider"`).
    Slider = 155,
    /// `AX_ROLE_SPINBUTTON`: numeric spinner (`<input type="number">` or `role="spinbutton"`).
    SpinButton = 156,
    /// `AX_ROLE_SPLITTER`: separator or resizable splitter (`<hr>` or `role="separator"`).
    Splitter = 157,
    /// `AX_ROLE_STATICTEXT`: static text node.
    StaticText = 158,
    /// `AX_ROLE_STATUS`: status live region (`<output>` or `role="status"`).
    Status = 159,
    /// `AX_ROLE_STRONG`: strong importance (`<strong>` or `role="strong"`).
    Strong = 160,
    /// `AX_ROLE_SUGGESTION`: editing suggestion annotation (`role="suggestion"`).
    Suggestion = 161,
    /// `AX_ROLE_SVGROOT`: root SVG graphic (`<svg>`).
    SvgRoot = 162,
    /// `AX_ROLE_SWITCH`: two-state toggle switch (`role="switch"`).
    Switch = 163,
    /// `AX_ROLE_TAB`: tab item (`role="tab"`).
    Tab = 164,
    /// `AX_ROLE_TABLIST`: tab list container (`role="tablist"`).
    TabList = 165,
    /// `AX_ROLE_TABPANEL`: tab panel content (`role="tabpanel"`).
    TabPanel = 166,
    /// `AX_ROLE_TABLE`: data table (`<table>` or `role="table"`).
    Table = 167,
    /// `AX_ROLE_TABLEHEADERCONTAINER`: container grouping table column headers.
    TableHeaderContainer = 168,
    /// `AX_ROLE_TERM`: defined term (`<dt>` or `role="term"`).
    Term = 169,
    /// `AX_ROLE_TEXTFIELD`: single-line or multi-line text input (`role="textbox"`).
    TextField = 170,
    /// `AX_ROLE_TEXTFIELDWITHCOMBOBOX`: editable text field paired with a combobox popup.
    TextFieldWithComboBox = 171,
    /// `AX_ROLE_TIME`: time value (`<time>` or `role="time"`).
    Time = 172,
    /// `AX_ROLE_TIMER`: timer live region (`role="timer"`).
    Timer = 173,
    /// `AX_ROLE_TITLEBAR`: window title bar.
    TitleBar = 174,
    /// `AX_ROLE_TOGGLEBUTTON`: toggleable button (`<button aria-pressed>`).
    ToggleButton = 175,
    /// `AX_ROLE_TOOLBAR`: toolbar container (`role="toolbar"`).
    Toolbar = 176,
    /// `AX_ROLE_TOOLTIP`: tooltip popup (`role="tooltip"`).
    Tooltip = 177,
    /// `AX_ROLE_TREE`: hierarchical tree widget (`role="tree"`).
    Tree = 178,
    /// `AX_ROLE_TREEGRID`: hierarchical grid widget (`role="treegrid"`).
    TreeGrid = 179,
    /// `AX_ROLE_TREEITEM`: node inside a tree widget (`role="treeitem"`).
    TreeItem = 180,
    /// `AX_ROLE_UNKNOWN`: unrecognized accessibility role.
    Unknown = 181,
    /// `AX_ROLE_VIDEO`: video media element (`<video>`).
    Video = 182,
    /// `AX_ROLE_WEBVIEW`: embedded web view container.
    WebView = 183,
    /// `AX_ROLE_WINDOW`: top-level UI window.
    Window = 184,
    /// `AX_ROLE_SUBSCRIPT`: subscript text (`<sub>` or `role="subscript"`).
    Subscript = 185,
    /// `AX_ROLE_SUPERSCRIPT`: superscript text (`<sup>` or `role="superscript"`).
    Superscript = 186,
    /// `AX_ROLE_MATHMLMATH`: `MathML` root `<math>` element.
    MathMlMath = 187,
    /// `AX_ROLE_MATHMLFRACTION`: `MathML` `<mfrac>` fraction element.
    MathMlFraction = 188,
    /// `AX_ROLE_MATHMLIDENTIFIER`: `MathML` `<mi>` identifier element.
    MathMlIdentifier = 189,
    /// `AX_ROLE_MATHMLMULTISCRIPTS`: `MathML` `<mmultiscripts>` element.
    MathMlMultiscripts = 190,
    /// `AX_ROLE_MATHMLNONESCRIPT`: `MathML` `<none>` placeholder script element.
    MathMlNoneScript = 191,
    /// `AX_ROLE_MATHMLNUMBER`: `MathML` `<mn>` number literal element.
    MathMlNumber = 192,
    /// `AX_ROLE_MATHMLOPERATOR`: `MathML` `<mo>` operator element.
    MathMlOperator = 193,
    /// `AX_ROLE_MATHMLOVER`: `MathML` `<mover>` overscript element.
    MathMlOver = 194,
    /// `AX_ROLE_MATHMLPRESCRIPTDELIMITER`: `MathML` `<mprescripts>` delimiter element.
    MathMlPrescriptDelimiter = 195,
    /// `AX_ROLE_MATHMLROOT`: `MathML` `<mroot>` radical with index element.
    MathMlRoot = 196,
    /// `AX_ROLE_MATHMLROW`: `MathML` `<mrow>` horizontal grouping element.
    MathMlRow = 197,
    /// `AX_ROLE_MATHMLSQUAREROOT`: `MathML` `<msqrt>` square-root element.
    MathMlSquareRoot = 198,
    /// `AX_ROLE_MATHMLSTRINGLITERAL`: `MathML` `<ms>` string literal element.
    MathMlStringLiteral = 199,
    /// `AX_ROLE_MATHMLSUB`: `MathML` `<msub>` subscript element.
    MathMlSub = 200,
    /// `AX_ROLE_MATHMLSUBSUP`: `MathML` `<msubsup>` subscript-superscript element.
    MathMlSubSup = 201,
    /// `AX_ROLE_MATHMLSUP`: `MathML` `<msup>` superscript element.
    MathMlSup = 202,
    /// `AX_ROLE_MATHMLTABLE`: `MathML` `<mtable>` matrix/table element.
    MathMlTable = 203,
    /// `AX_ROLE_MATHMLTABLECELL`: `MathML` `<mtd>` table cell element.
    MathMlTableCell = 204,
    /// `AX_ROLE_MATHMLTABLEROW`: `MathML` `<mtr>` table row element.
    MathMlTableRow = 205,
    /// `AX_ROLE_MATHMLTEXT`: `MathML` `<mtext>` text element.
    MathMlText = 206,
    /// `AX_ROLE_MATHMLUNDER`: `MathML` `<munder>` underscript element.
    MathMlUnder = 207,
    /// `AX_ROLE_MATHMLUNDEROVER`: `MathML` `<munderover>` under-overscript element.
    MathMlUnderOver = 208,
    /// `AX_ROLE_COMBOBOXSELECT`: native `<select>` styled as a combobox.
    ComboBoxSelect = 209,
    /// `AX_ROLE_DISCLOSURETRIANGLEGROUPED`: grouped `<details name="...">` disclosure toggle.
    DisclosureTriangleGrouped = 210,
    /// `AX_ROLE_SECTIONWITHOUTNAME`: unnamed `<section>` element.
    SectionWithoutName = 211,
    /// `AX_ROLE_GRIDCELL`: interactive grid cell (`role="gridcell"`).
    GridCell = 212,
}
