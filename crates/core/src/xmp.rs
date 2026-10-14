//! Removes `photoshop:DocumentAncestors` from an XMP packet (image resource 1060).
//!
//! The packet is parsed with roxmltree only to locate the property by namespace; the edit is a
//! byte splice of the property's element (and the whitespace line it sat on), so every other byte
//! of the packet, including other metadata and formatting, is kept.

use crate::error::{Error, Result};

/// The Photoshop XMP namespace (`XMPConst.NS_PHOTOSHOP`).
pub const PHOTOSHOP_NS: &str = "http://ns.adobe.com/photoshop/1.0/";
const PROPERTY: &str = "DocumentAncestors";

/// The result of a successful removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmpEdit {
    pub data: Vec<u8>,
    /// Number of `rdf:li` entries removed.
    pub entries: usize,
}

/// Counts the ancestor entries without changing anything (`0` when the property is absent).
pub fn count_document_ancestors(data: &[u8]) -> Result<usize> {
    Ok(find(data)?.map_or(0, |(_, n)| n))
}

/// Removes every `photoshop:DocumentAncestors` property. `None` when there is none.
pub fn remove_document_ancestors(data: &[u8]) -> Result<Option<XmpEdit>> {
    let Some((ranges, entries)) = find(data)? else { return Ok(None) };
    let mut out = Vec::with_capacity(data.len());
    let mut at = 0;
    for r in ranges {
        let start = line_start(data, r.start);
        out.extend_from_slice(data.get(at..start).unwrap_or_default());
        at = r.end;
    }
    out.extend_from_slice(data.get(at..).unwrap_or_default());
    Ok(Some(XmpEdit { data: out, entries }))
}

/// Byte ranges of the property elements and the number of entries, or `None` when absent.
fn find(data: &[u8]) -> Result<Option<(Vec<std::ops::Range<usize>>, usize)>> {
    if !contains(data, PROPERTY.as_bytes()) {
        return Ok(None);
    }
    let text = std::str::from_utf8(data).map_err(|e| Error::format("XMP", format!("not UTF-8: {e}")))?;
    // Packets may be followed by NUL or whitespace padding, which XML does not allow after the
    // root's trailing processing instruction; parse without it (offsets stay valid).
    let text = text.trim_end_matches(['\0', ' ', '\t', '\r', '\n']);
    let opts = roxmltree::ParsingOptions { allow_dtd: false, ..Default::default() };
    let doc = roxmltree::Document::parse_with_options(text, opts).map_err(|e| Error::format("XMP", e))?;
    let mut ranges = Vec::new();
    let mut entries = 0;
    for node in doc.descendants() {
        let name = node.tag_name();
        if node.is_element() && name.name() == PROPERTY && name.namespace() == Some(PHOTOSHOP_NS) {
            // Nested ancestors inside another ancestors element are covered by the outer range.
            if ranges.iter().any(|r: &std::ops::Range<usize>| r.contains(&node.range().start)) {
                continue;
            }
            entries += node.descendants().filter(|n| n.is_element() && n.tag_name().name() == "li").count();
            ranges.push(node.range());
        }
    }
    if ranges.is_empty() {
        return Ok(None);
    }
    ranges.sort_by_key(|r| r.start);
    Ok(Some((ranges, entries)))
}

/// Extends `start` back over the indentation before it, including the preceding newline, when
/// only whitespace separates them; otherwise returns `start`.
fn line_start(data: &[u8], start: usize) -> usize {
    let mut i = start;
    while i > 0 && matches!(data.get(i - 1), Some(b' ' | b'\t')) {
        i -= 1;
    }
    match (i.checked_sub(1).and_then(|j| data.get(j)), i.checked_sub(2).and_then(|j| data.get(j))) {
        (Some(b'\n'), Some(b'\r')) => i - 2,
        (Some(b'\n' | b'\r'), _) => i - 1,
        _ => start,
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests {
    use super::*;

    /// A Photoshop-style packet with `n` ancestors (or none).
    pub fn packet(n: Option<usize>) -> String {
        let anc = match n {
            None => String::new(),
            Some(n) => {
                let lis: String = (0..n).map(|i| format!("\n               <rdf:li>adobe:docid:photoshop:{i:08x}</rdf:li>")).collect();
                format!("\n         <photoshop:DocumentAncestors>\n            <rdf:Bag>{lis}\n            </rdf:Bag>\n         </photoshop:DocumentAncestors>")
            }
        };
        format!(
            "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"Adobe XMP Core 9.1\">\n   <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n      <rdf:Description rdf:about=\"\"\n            xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\"\n            xmlns:photoshop=\"http://ns.adobe.com/photoshop/1.0/\">\n         <xmp:CreatorTool>Adobe Photoshop 25.0</xmp:CreatorTool>\n         <photoshop:ColorMode>3</photoshop:ColorMode>{anc}\n         <photoshop:ICCProfile>sRGB IEC61966-2.1</photoshop:ICCProfile>\n      </rdf:Description>\n   </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>"
        )
    }

    #[test]
    fn removes_only_the_property() {
        let with = packet(Some(3));
        let edit = remove_document_ancestors(with.as_bytes()).unwrap().unwrap();
        assert_eq!(edit.entries, 3);
        assert_eq!(String::from_utf8(edit.data).unwrap(), packet(None));
        assert_eq!(count_document_ancestors(with.as_bytes()).unwrap(), 3);
    }

    #[test]
    fn absent_is_none_and_empty_bag_is_removed() {
        assert_eq!(remove_document_ancestors(packet(None).as_bytes()).unwrap(), None);
        let edit = remove_document_ancestors(packet(Some(0)).as_bytes()).unwrap().unwrap();
        assert_eq!(edit.entries, 0);
        assert_eq!(edit.data, packet(None).as_bytes());
    }

    #[test]
    fn matches_by_namespace_not_prefix() {
        let other = packet(Some(2)).replace("xmlns:photoshop=\"http://ns.adobe.com/photoshop/1.0/\"", "xmlns:photoshop=\"http://example.com/\"");
        assert_eq!(remove_document_ancestors(other.as_bytes()).unwrap(), None);
        let renamed = packet(Some(2)).replace("photoshop:", "ps:").replace("xmlns:photoshop=", "xmlns:ps=");
        let edit = remove_document_ancestors(renamed.as_bytes()).unwrap().unwrap();
        assert!(!String::from_utf8(edit.data).unwrap().contains("DocumentAncestors"));
    }

    #[test]
    fn trailing_padding_and_self_closing() {
        let mut p = packet(Some(1)).into_bytes();
        p.extend([b' '; 10]);
        p.extend([0; 3]);
        let edit = remove_document_ancestors(&p).unwrap().unwrap();
        assert!(edit.data.ends_with(&[b' ', b' ', 0, 0, 0]));
        let sc = packet(None).replace("<photoshop:ColorMode>", "<photoshop:DocumentAncestors/>\n         <photoshop:ColorMode>");
        let edit = remove_document_ancestors(sc.as_bytes()).unwrap().unwrap();
        assert_eq!(edit.data, packet(None).as_bytes());
    }

    #[test]
    fn malformed_is_an_error() {
        let bad = packet(Some(1)).replace("</rdf:Bag>", "");
        assert!(remove_document_ancestors(bad.as_bytes()).is_err());
        assert!(remove_document_ancestors(b"\xff DocumentAncestors").is_err());
    }
}
