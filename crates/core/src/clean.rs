//! Recursive cleaner: strips `photoshop:DocumentAncestors` from a document and from every
//! embedded PSD/PSB smart object inside it, at any depth, and splices the cleaned children back
//! into their parents.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::linked::{self, Embedded};
use crate::psd::{self, Edits, Layout};
use crate::xmp;

const XMP_RESOURCE: u16 = 1060;
const LINK_KEYS: [&[u8; 4]; 3] = [b"lnk2", b"lnk3", b"lnkD"];

/// Cleaner settings.
#[derive(Clone, Debug)]
pub struct Options {
    /// Maximum smart-object nesting depth (the top document is depth 0).
    pub max_depth: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options { max_depth: 32 }
    }
}

/// What a run found and did.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Stats {
    /// PSD/PSB documents visited, the top document included (shared copies counted once).
    pub documents: usize,
    /// Documents whose bytes changed.
    pub documents_changed: usize,
    /// Embedded PSD/PSB smart objects found (every occurrence).
    pub smart_objects: usize,
    /// Smart objects whose content was identical to one already cleaned.
    pub shared_smart_objects: usize,
    /// `rdf:li` ancestor entries removed.
    pub ancestors_removed: usize,
    /// Embedded files that are not PSD/PSB, by file type (left untouched).
    pub skipped_types: BTreeMap<String, usize>,
    /// Non-fatal problems: parts left untouched.
    pub warnings: Vec<String>,
}

/// Progress notifications.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// Started cleaning an embedded smart object.
    Enter { depth: usize, name: String },
    /// Finished one; `ancestors` removed from it and everything nested in it.
    Leave { depth: usize, name: String, ancestors: usize, changed: bool },
    /// An identical smart object was already cleaned; its result is reused.
    Shared { depth: usize, name: String },
    /// Share of the top document processed so far (0..1), by the byte position of the last
    /// top-level smart object finished.
    Progress { fraction: f64 },
}

/// One cleaning run over a top-level document.
pub struct Cleaner<'a> {
    opts: Options,
    cancel: Option<&'a AtomicBool>,
    on_event: Option<&'a mut dyn FnMut(&Event)>,
    cache: HashMap<[u8; 32], Option<Arc<Vec<u8>>>>,
    pub stats: Stats,
}

impl<'a> Cleaner<'a> {
    pub fn new(opts: Options) -> Self {
        Cleaner { opts, cancel: None, on_event: None, cache: HashMap::new(), stats: Stats::default() }
    }

    pub fn with_cancel(mut self, flag: &'a AtomicBool) -> Self {
        self.cancel = Some(flag);
        self
    }

    pub fn with_events(mut self, f: &'a mut dyn FnMut(&Event)) -> Self {
        self.on_event = Some(f);
        self
    }

    fn emit(&mut self, e: Event) {
        if let Some(f) = self.on_event.as_mut() {
            f(&e);
        }
    }

    fn check_cancel(&self) -> Result<()> {
        match self.cancel {
            Some(c) if c.load(Ordering::Relaxed) => Err(Error::Cancelled),
            _ => Ok(()),
        }
    }

    /// Cleans a whole document. `Ok(None)` when nothing needed to change.
    pub fn clean(&mut self, bytes: &[u8]) -> Result<Option<Vec<u8>>> {
        self.clean_doc(bytes, 0, "")
    }

    fn clean_doc(&mut self, bytes: &[u8], depth: usize, name: &str) -> Result<Option<Vec<u8>>> {
        self.check_cancel()?;
        let layout = Layout::parse(bytes)?;
        self.stats.documents += 1;
        let label = if name.is_empty() { "document" } else { name };
        let mut edits = Edits::default();

        for (i, res) in layout.resources.iter().enumerate().filter(|(_, r)| r.id == XMP_RESOURCE) {
            let data = bytes.get(res.data.clone()).unwrap_or_default();
            match xmp::remove_document_ancestors(data) {
                Ok(Some(e)) => {
                    self.stats.ancestors_removed += e.entries;
                    edits.resources.push((i, e.data));
                }
                Ok(None) => {}
                Err(e) => self.stats.warnings.push(format!("{label}: XMP left as is: {e}")),
            }
        }

        for (bi, block) in layout.blocks.iter().enumerate().filter(|(_, b)| LINK_KEYS.contains(&&b.key)) {
            let data = bytes.get(block.data.clone()).unwrap_or_default();
            let items = match linked::split_items(data) {
                Ok(items) => items,
                Err(e) => {
                    self.stats.warnings.push(format!("{label}: {} block left as is: {e}", String::from_utf8_lossy(&block.key)));
                    continue;
                }
            };
            let mut replacements: Vec<(Embedded, Arc<Vec<u8>>)> = Vec::new();
            for (ii, item) in items.iter().enumerate().filter(|(_, it)| &it.kind == b"liFD") {
                let e = match linked::parse_embedded(data, ii, item) {
                    Ok(e) => e,
                    Err(err) => {
                        self.stats.warnings.push(format!("{label}: embedded file {:?} left as is: {err}", String::from_utf8_lossy(&item.uuid)));
                        continue;
                    }
                };
                let payload = data.get(e.payload.clone()).unwrap_or_default();
                if !psd::is_psd(payload) {
                    *self.stats.skipped_types.entry(file_type_name(&e.file_type, payload)).or_default() += 1;
                    continue;
                }
                self.stats.smart_objects += 1;
                if depth + 1 > self.opts.max_depth {
                    self.stats.warnings.push(format!("{}: nested deeper than {} levels, left as is", e.file_name, self.opts.max_depth));
                    continue;
                }
                let key: [u8; 32] = blake3::hash(payload).into();
                let cleaned = match self.cache.get(&key).cloned() {
                    Some(c) => {
                        self.stats.shared_smart_objects += 1;
                        self.emit(Event::Shared { depth: depth + 1, name: e.file_name.clone() });
                        c
                    }
                    None => {
                        self.emit(Event::Enter { depth: depth + 1, name: e.file_name.clone() });
                        let before = self.stats.ancestors_removed;
                        let c = match self.clean_doc(payload, depth + 1, &e.file_name) {
                            Ok(c) => c.map(Arc::new),
                            Err(Error::Cancelled) => return Err(Error::Cancelled),
                            Err(err) => {
                                self.stats.warnings.push(format!("{}: left as is: {err}", e.file_name));
                                None
                            }
                        };
                        let ancestors = self.stats.ancestors_removed - before;
                        self.emit(Event::Leave { depth: depth + 1, name: e.file_name.clone(), ancestors, changed: c.is_some() });
                        self.cache.insert(key, c.clone());
                        c
                    }
                };
                if depth == 0 && !bytes.is_empty() {
                    let fraction = (block.data.start + e.payload.end) as f64 / bytes.len() as f64;
                    self.emit(Event::Progress { fraction: fraction.min(1.0) });
                }
                if let Some(c) = cleaned {
                    replacements.push((e, c));
                }
            }
            if !replacements.is_empty() {
                let refs: Vec<(&Embedded, &[u8])> = replacements.iter().map(|(e, c)| (e, c.as_slice())).collect();
                edits.blocks.push((bi, linked::replace_payloads(data, &items, &refs)?));
            }
        }

        if edits.is_empty() {
            return Ok(None);
        }
        self.check_cancel()?;
        let out = psd::rebuild(bytes, &layout, &edits)?;
        self.stats.documents_changed += 1;
        Ok(Some(out))
    }
}

fn file_type_name(ft: &[u8; 4], payload: &[u8]) -> String {
    let t = String::from_utf8_lossy(ft).trim().to_string();
    if !t.is_empty() {
        return t;
    }
    match payload {
        [0x89, b'P', b'N', b'G', ..] => "PNG".into(),
        [0xFF, 0xD8, ..] => "JPEG".into(),
        [b'%', b'P', b'D', b'F', ..] => "PDF".into(),
        _ => "unknown".into(),
    }
}

/// A structural summary of a document tree, used to verify a cleaned file before it is written.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    /// Distinct PSD/PSB documents (by content).
    pub documents: usize,
    /// Embedded PSD/PSB occurrences.
    pub smart_objects: usize,
    /// Ancestor entries still present.
    pub ancestors: usize,
    /// Parts that could not be read.
    pub errors: usize,
}

/// Walks a document tree read-only.
pub fn summarize(bytes: &[u8], opts: &Options) -> Result<Summary> {
    let mut s = Summary::default();
    let mut seen = std::collections::HashSet::new();
    walk(bytes, 0, opts, &mut s, &mut seen)?;
    Ok(s)
}

fn walk(bytes: &[u8], depth: usize, opts: &Options, s: &mut Summary, seen: &mut std::collections::HashSet<[u8; 32]>) -> Result<()> {
    let layout = Layout::parse(bytes)?;
    s.documents += 1;
    for res in layout.resources.iter().filter(|r| r.id == XMP_RESOURCE) {
        match xmp::count_document_ancestors(bytes.get(res.data.clone()).unwrap_or_default()) {
            Ok(n) => s.ancestors += n,
            Err(_) => s.errors += 1,
        }
    }
    for block in layout.blocks.iter().filter(|b| LINK_KEYS.contains(&&b.key)) {
        let data = bytes.get(block.data.clone()).unwrap_or_default();
        let Ok(items) = linked::split_items(data) else {
            s.errors += 1;
            continue;
        };
        for (ii, item) in items.iter().enumerate().filter(|(_, it)| &it.kind == b"liFD") {
            let Ok(e) = linked::parse_embedded(data, ii, item) else {
                s.errors += 1;
                continue;
            };
            let payload = data.get(e.payload.clone()).unwrap_or_default();
            if !psd::is_psd(payload) {
                continue;
            }
            s.smart_objects += 1;
            if depth + 1 > opts.max_depth || !seen.insert(blake3::hash(payload).into()) {
                continue;
            }
            if walk(payload, depth + 1, opts, s, seen).is_err() {
                s.errors += 1;
            }
        }
    }
    Ok(())
}
