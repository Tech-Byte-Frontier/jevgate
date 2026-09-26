//! Deserializers that can build any object and XML parsers that can resolve
//! external entities: the names that make code worth asking about them, and
//! the checks it is then asked.
use super::Check;

/// Whether data another party sends is loaded with a deserializer that can
/// build any object or run code, in Python's terms.
pub(super) const DESERIALIZE: Check = Check {
    id: "deserialize",
    question: "Does `{code}` load data that another party can send with a deserializer that can build any object or run code?",
    yes: "Request data, an uploaded file, a cookie, a message or a stored value users can set is passed to pickle, marshal, shelve, jsonpickle, yaml.load without a safe loader, or a similar deserializer.",
    no: "It parses JSON, uses yaml.safe_load or another data-only format, or loads only data the program wrote and signed itself.",
    no_examples: &[],
};

/// The deserialize check in the terms of a language other than Python.
const fn deserialize(yes: &'static str, no: &'static str) -> Check {
    Check {
        id: "deserialize",
        question: DESERIALIZE.question,
        yes,
        no,
        no_examples: &[],
    }
}

/// Deserializers that can build any object or run code, per language, as
/// (language, what its source must name in any case, how the presence
/// question names them, the trace check). Django asks its own check of
/// every view and PHP of source that names `unserialize`; other code was
/// never asked, so a Flask route passing `pickle.loads(request.get_data())`
/// was clear. The question and check are added only to source that names
/// one, so every other request, and its cached answer, stays as it was.
const DESERIALIZERS: [(&str, &[&str], &str, Check); 5] = [
    (
        "Python",
        &[
            "pickle.load",
            "pickle.unpickler",
            "marshal.load",
            "shelve.open",
            "jsonpickle.decode",
            "dill.load",
            "yaml.load(",
            "yaml.unsafe_load",
            "yaml.full_load",
            "yaml.load_all(",
        ],
        "pickle, marshal, shelve, jsonpickle or yaml.load without a safe loader",
        DESERIALIZE,
    ),
    (
        "Ruby",
        &[
            "marshal.load",
            "yaml.load",
            "yaml.unsafe_load",
            "psych.load",
            "oj.load",
        ],
        "Marshal.load, YAML.unsafe_load or Oj.load in object mode",
        deserialize(
            "Request data, an uploaded file, a cookie, a message or a stored value users can set is passed to Marshal.load, YAML.unsafe_load, YAML.load with unsafe options, Oj.load in object mode or a similar deserializer.",
            "It parses JSON, uses YAML.safe_load or another data-only format, or loads only data the program wrote and signed itself.",
        ),
    ),
    (
        "Java",
        &[
            "objectinputstream",
            "xmldecoder",
            "fromxml(",
            "enabledefaulttyping",
            "activatedefaulttyping",
            "new yaml(",
        ],
        "ObjectInputStream, XMLDecoder, XStream or SnakeYAML's Yaml.load",
        deserialize(
            "Request data, an uploaded file, a cookie, a message or a stored value users can set is read with ObjectInputStream.readObject, XMLDecoder, XStream.fromXML, SnakeYAML's Yaml.load, Jackson with default typing enabled or a similar deserializer.",
            "It parses JSON into types fixed in the code, uses a safe constructor or an allowed list of classes, or loads only data the program wrote and signed itself.",
        ),
    ),
    (
        "JavaScript",
        &["node-serialize", "unserialize(", "funcster", "cryo.parse"],
        "node-serialize's unserialize, funcster or cryo",
        NODE_DESERIALIZE,
    ),
    (
        "TypeScript",
        &["node-serialize", "unserialize(", "funcster", "cryo.parse"],
        "node-serialize's unserialize, funcster or cryo",
        NODE_DESERIALIZE,
    ),
];

const NODE_DESERIALIZE: Check = deserialize(
    "Request data, a cookie, a message or a stored value users can set is passed to node-serialize's unserialize, funcster, cryo or a similar deserializer that can restore functions.",
    "It parses JSON with JSON.parse or another data-only format, or loads only data the program wrote and signed itself.",
);

/// The deserializer entry of `language` whose names `source` holds.
fn deserializer_entry(
    language: &str,
    source: &str,
) -> Option<&'static (&'static str, &'static [&'static str], &'static str, Check)> {
    let source = source.to_ascii_lowercase();
    DESERIALIZERS.iter().find(|(lang, names, ..)| {
        *lang == language && names.iter().any(|name| source.contains(name))
    })
}

/// How the presence question names `language`'s deserializers, when
/// `source` names one of them.
pub fn deserializers_named(language: &str, source: &str) -> Option<&'static str> {
    deserializer_entry(language, source).map(|(_, _, names, _)| *names)
}

/// The deserialize check asked of `source` in `language`, when it names one
/// of the language's deserializers.
pub fn deserializer_check(language: &str, source: &str) -> Option<&'static Check> {
    deserializer_entry(language, source).map(|(.., check)| check)
}

/// XML parsers that can resolve external entities or load document type
/// definitions, as source names them in any case: lxml, Python's pulldom
/// and SAX, Java's DocumentBuilderFactory, SAXParserFactory,
/// XMLInputFactory, TransformerFactory, dom4j and JDOM, .NET's XmlDocument
/// and XmlTextReader, PHP's SimpleXML and DOMDocument, libxmljs and
/// Nokogiri. A pygoat lab parsing a request with lxml and a Spring
/// controller parsing its body with a default DocumentBuilderFactory were
/// never asked about entities.
const XML_PARSERS: [&str; 18] = [
    "lxml",
    "resolve_entities",
    "pulldom",
    "xml.sax",
    "documentbuilderfactory",
    "saxparserfactory",
    "xmlinputfactory",
    "transformerfactory",
    "saxreader",
    "saxbuilder",
    "xmldocument",
    "xmltextreader",
    "dtdprocessing",
    "simplexml_load",
    "domdocument",
    "libxml_noent",
    "libxmljs",
    "nokogiri",
];

/// Whether `source` names an XML parser that can resolve external entities.
fn xml_parser_named(source: &str) -> bool {
    let source = source.to_ascii_lowercase();
    XML_PARSERS.iter().any(|name| source.contains(name))
}

/// Calls that parse XML with a parser created or imported elsewhere in the
/// file: `make_parser()`, `parseString(…)`, `etree.fromstring(…)`.
const XML_CALLS: [&str; 5] = ["parse", "fromstring", "iterparse", "expandnode", "xml("];

/// Whether `code`, in a file whose whole source is `file`, parses XML with a
/// parser that can resolve external entities: it names one itself, or its
/// file imports one and it calls a parse method. Python modules import
/// lxml or `xml.sax` at the top, so pygoat's lab calling `make_parser()` and
/// `parseString(…)` named no parser in its own source.
pub fn parses_xml(file: &str, code: &str) -> bool {
    xml_parser_named(code)
        || (xml_parser_named(file) && {
            let code = code.to_ascii_lowercase();
            XML_CALLS.iter().any(|call| code.contains(call))
        })
}

/// Whether XML from another party is parsed with external entities enabled,
/// asked only of source that names such a parser.
pub const XXE: Check = Check {
    id: "xxe",
    question: "Does `{code}` parse XML that another party can send with a parser that resolves external entities or loads document type definitions?",
    yes: "Request data, an upload or a message is parsed as XML with external entities, DTD loading or entity substitution enabled, or with a parser whose defaults allow them, such as Java's DocumentBuilderFactory, SAXParserFactory or XMLInputFactory without disallowing DOCTYPE declarations, lxml with resolve_entities or load_dtd, .NET's XmlDocument with an XmlResolver or DtdProcessing.Parse, PHP's LIBXML_NOENT, or libxmljs with noent.",
    no: "It turns DOCTYPE declarations and external entities off (disallow-doctype-decl, resolve_entities=False, DtdProcessing.Prohibit, a null XmlResolver), uses a parser that never resolves them such as Python's xml.etree or defusedxml, or parses only XML the program wrote itself.",
    no_examples: &[],
};
