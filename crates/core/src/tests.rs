//! Integration tests over synthetic PSD/PSB trees.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use std::sync::atomic::AtomicBool;

use crate::clean::{summarize, Cleaner, Options};
use crate::linked::tests::Rec;
use crate::psd::Layout;
use crate::xmp::tests::packet;

/// A synthetic document.
#[derive(Clone)]
pub struct Doc {
    pub psb: bool,
    pub xmp: Option<String>,
    pub children: Vec<Rec>,
    /// Use `lnk3` instead of `lnk2` for the children.
    pub key: [u8; 4],
}

impl Doc {
    pub fn new(psb: bool, ancestors: Option<usize>) -> Self {
        Doc { psb, xmp: Some(packet(ancestors)), children: vec![], key: *b"lnk2" }
    }

    pub fn child(mut self, uuid: &str, payload: Vec<u8>) -> Self {
        self.children.push(Rec::new(uuid, payload));
        self
    }

    fn block(&self, out: &mut Vec<u8>, key: &[u8; 4], data: &[u8], pad: &[u8]) {
        out.extend_from_slice(b"8BIM");
        out.extend_from_slice(key);
        let long = self.psb && matches!(key, b"lnk2" | b"lnk3");
        if long {
            out.extend((data.len() as u64).to_be_bytes());
        } else {
            out.extend((data.len() as u32).to_be_bytes());
        }
        out.extend_from_slice(data);
        out.extend_from_slice(pad);
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut o = b"8BPS".to_vec();
        o.extend((if self.psb { 2u16 } else { 1 }).to_be_bytes());
        o.extend([0; 6]);
        o.extend(3u16.to_be_bytes()); // channels
        o.extend(2u32.to_be_bytes()); // height
        o.extend(2u32.to_be_bytes()); // width
        o.extend(8u16.to_be_bytes());
        o.extend(3u16.to_be_bytes()); // RGB
        o.extend(0u32.to_be_bytes()); // color mode data

        let mut res = Vec::new();
        // Resource with a name and odd data.
        res.extend_from_slice(b"8BIM");
        res.extend(1000u16.to_be_bytes());
        res.extend([3, b'a', b'b', b'c']);
        res.extend(3u32.to_be_bytes());
        res.extend([1, 2, 3, 0]);
        if let Some(x) = &self.xmp {
            res.extend_from_slice(b"8BIM");
            res.extend(1060u16.to_be_bytes());
            res.extend([0, 0]);
            res.extend((x.len() as u32).to_be_bytes());
            res.extend(x.as_bytes());
            if x.len() % 2 == 1 {
                res.push(0);
            }
        }
        res.extend_from_slice(b"8BIM");
        res.extend(1057u16.to_be_bytes());
        res.extend([0, 0]);
        res.extend(4u32.to_be_bytes());
        res.extend([0, 0, 0, 1]);
        o.extend((res.len() as u32).to_be_bytes());
        o.extend(res);

        let lw = |v: &mut Vec<u8>, n: usize| {
            if self.psb {
                v.extend((n as u64).to_be_bytes())
            } else {
                v.extend((n as u32).to_be_bytes())
            }
        };
        let mut lm = Vec::new();
        let layer_info = vec![0xA5u8; 22];
        lw(&mut lm, layer_info.len());
        lm.extend(&layer_info);
        lm.extend(4u32.to_be_bytes());
        lm.extend([9, 9, 9, 9]);
        self.block(&mut lm, b"BEFR", &[1, 2, 3, 4, 5], &[0, 0, 0]);
        if !self.children.is_empty() {
            let data: Vec<u8> = self.children.iter().flat_map(Rec::encode).collect();
            self.block(&mut lm, &self.key, &data, &[]);
        }
        self.block(&mut lm, b"AFTR", &[7; 6], &[0, 0]);
        let mut sec = Vec::new();
        lw(&mut sec, lm.len());
        o.extend(sec);
        o.extend(lm);
        o.extend(0u16.to_be_bytes());
        o.extend([0x11; 12]);
        o
    }
}

fn region(bytes: &[u8], key: &[u8; 4]) -> Vec<u8> {
    let l = Layout::parse(bytes).unwrap();
    let b = l.blocks.iter().find(|b| &b.key == key).unwrap();
    bytes[b.span.clone()].to_vec()
}

fn tree(psb: bool) -> Vec<u8> {
    let grandchild = Doc::new(false, Some(5)).encode();
    let child = Doc::new(true, Some(4)).child("g1", grandchild.clone()).child("png", vec![0x89, b'P', b'N', b'G', 1, 2]).encode();
    let mut doc = Doc::new(psb, Some(3)).child("c1", child.clone()).child("c2", child);
    doc.children[1].name = "copy.psb".into();
    doc.encode()
}

#[test]
fn cleans_every_level_and_keeps_other_bytes() {
    for psb in [false, true] {
        let input = tree(psb);
        assert_eq!(summarize(&input, &Options::default()).unwrap().ancestors, 3 + 4 + 5);
        let mut c = Cleaner::new(Options::default());
        let out = c.clean(&input).unwrap().unwrap();
        assert_eq!(c.stats.ancestors_removed, 3 + 4 + 5, "shared child counted once");
        assert_eq!(c.stats.smart_objects, 3);
        assert_eq!(c.stats.shared_smart_objects, 1);
        assert_eq!(c.stats.skipped_types.get("8BPB"), Some(&1), "the PNG record says 8BPB in the fixture");
        assert!(c.stats.warnings.is_empty(), "{:?}", c.stats.warnings);
        let s = summarize(&out, &Options::default()).unwrap();
        assert_eq!((s.ancestors, s.errors, s.smart_objects), (0, 0, 3));
        assert!(out.len() < input.len());
        // Untouched parts are byte-identical.
        assert_eq!(region(&out, b"BEFR"), region(&input, b"BEFR"));
        assert_eq!(region(&out, b"AFTR"), region(&input, b"AFTR"));
        assert!(out.ends_with(&[0, 0, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11]));
        // The expected output is the same tree built clean.
        let grandchild = Doc::new(false, None).encode();
        let child = Doc::new(true, None).child("g1", grandchild).child("png", vec![0x89, b'P', b'N', b'G', 1, 2]).encode();
        let mut doc = Doc::new(psb, None).child("c1", child.clone()).child("c2", child);
        doc.children[1].name = "copy.psb".into();
        assert_eq!(out, doc.encode());
        // Idempotent.
        assert!(Cleaner::new(Options::default()).clean(&out).unwrap().is_none());
    }
}

#[test]
fn lnk3_and_documents_without_xmp() {
    let mut doc = Doc::new(true, None).child("x", Doc::new(false, Some(2)).encode());
    doc.key = *b"lnk3";
    doc.xmp = None;
    let out = Cleaner::new(Options::default()).clean(&doc.encode()).unwrap().unwrap();
    assert_eq!(summarize(&out, &Options::default()).unwrap().ancestors, 0);
}

#[test]
fn depth_limit_leaves_deep_children() {
    let deep = Doc::new(false, Some(1)).child("d", Doc::new(false, Some(1)).encode()).encode();
    let mut c = Cleaner::new(Options { max_depth: 1 });
    let out = c.clean(&Doc::new(false, Some(1)).child("top", deep).encode()).unwrap().unwrap();
    assert_eq!(c.stats.ancestors_removed, 2);
    assert_eq!(c.stats.warnings.len(), 1);
    assert_eq!(summarize(&out, &Options::default()).unwrap().ancestors, 1);
}

#[test]
fn broken_child_is_left_as_is_with_a_warning() {
    let mut broken = Doc::new(false, Some(1)).encode();
    broken.truncate(40);
    let mut c = Cleaner::new(Options::default());
    let input = Doc::new(false, Some(2)).child("b", broken.clone()).encode();
    let out = c.clean(&input).unwrap().unwrap();
    assert_eq!(c.stats.warnings.len(), 1);
    assert!(out.windows(broken.len()).any(|w| w == broken));
}

#[test]
fn cancel_stops_the_run() {
    let flag = AtomicBool::new(true);
    let mut c = Cleaner::new(Options::default()).with_cancel(&flag);
    assert!(matches!(c.clean(&tree(false)), Err(crate::Error::Cancelled)));
}

#[test]
fn truncated_and_corrupted_files_never_panic() {
    let input = tree(true);
    for n in (0..input.len()).step_by(7) {
        let _ = Cleaner::new(Options::default()).clean(&input[..n]);
        let _ = summarize(&input[..n], &Options::default());
    }
    let mut x = 0x1234_5678u32;
    for _ in 0..300 {
        let mut b = input.clone();
        for _ in 0..4 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let i = x as usize % b.len();
            b[i] = (x >> 8) as u8;
        }
        let _ = Cleaner::new(Options::default()).clean(&b);
    }
}

#[test]
fn events_report_the_tree() {
    let mut names = Vec::new();
    let mut f = |e: &crate::Event| {
        if !matches!(e, crate::Event::Progress { .. }) {
            names.push(format!("{e:?}"))
        }
    };
    let mut c = Cleaner::new(Options::default()).with_events(&mut f);
    c.clean(&tree(false)).unwrap();
    drop(c);
    assert_eq!(names.len(), 5, "{names:#?}"); // enter/leave c1, enter/leave g1, shared c2
}

mod job {
    use super::*;
    use crate::job::{process, scan, Original, OutputRule, Plan, Status, Task, BACKUP_DIR};
    use std::fs;
    use std::path::{Path, PathBuf};

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("exif-cleaner-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn run(task: &Task, plan: Plan) -> crate::FileReport {
        process(task, &plan, &Options::default(), None, &mut |_| {})
    }

    fn ancestors(p: &Path) -> usize {
        summarize(&fs::read(p).unwrap(), &Options::default()).unwrap().ancestors
    }

    #[test]
    fn overwrite_dry_run_missing_and_backup_folder() {
        let d = tmpdir("job");
        let input = tree(false);
        fs::write(d.join("a.psd"), &input).unwrap();
        fs::create_dir_all(d.join("sub")).unwrap();
        fs::write(d.join("sub").join("b.PSB"), Doc::new(true, None).encode()).unwrap();
        fs::write(d.join("note.txt"), b"x").unwrap();
        let tasks = scan(&[d.clone()]);
        assert_eq!(tasks.len(), 2);

        let r = run(&tasks[0], Plan { dry_run: true, ..Default::default() });
        assert_eq!(r.status, Status::Cleaned);
        assert!(r.size_after.unwrap() < r.size_before);
        assert_eq!(fs::read(d.join("a.psd")).unwrap(), input, "dry run writes nothing");

        // Plain overwrite.
        let r = run(&tasks[0], Plan::default());
        assert_eq!(r.status, Status::Cleaned, "{:?}", r.error);
        assert_eq!(ancestors(&d.join("a.psd")), 0);
        assert_eq!(r.size_after, Some(fs::metadata(d.join("a.psd")).unwrap().len()));
        assert!(!d.join(BACKUP_DIR).exists());
        assert_eq!(run(&tasks[0], Plan::default()).status, Status::AlreadyClean);
        assert_eq!(run(&tasks[1], Plan::default()).status, Status::AlreadyClean);

        // Overwrite moving the original to the backup folder.
        fs::write(d.join("a.psd"), &input).unwrap();
        let r = run(&tasks[0], Plan { original: Original::BackupFolder, ..Default::default() });
        assert_eq!(r.status, Status::Cleaned, "{:?}", r.error);
        assert_eq!(fs::read(d.join(BACKUP_DIR).join("a.psd")).unwrap(), input);
        assert_eq!(ancestors(&d.join("a.psd")), 0);
        assert_eq!(scan(&[d.clone()]).len(), 2, "backups are not rescanned");

        fs::remove_file(d.join("a.psd")).unwrap();
        assert_eq!(run(&tasks[0], Plan::default()).status, Status::Missing);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn output_paths() {
        let d = tmpdir("out");
        let input = tree(false);
        fs::create_dir_all(d.join("in").join("deep")).unwrap();
        fs::write(d.join("in").join("deep").join("c.psd"), &input).unwrap();
        fs::write(d.join("in").join("clean.psd"), Doc::new(false, None).encode()).unwrap();
        let tasks = scan(&[d.join("in")]);
        assert_eq!(tasks.len(), 2);
        let c = tasks.iter().find(|t| t.path.ends_with("c.psd")).unwrap();
        let clean = tasks.iter().find(|t| t.path.ends_with("clean.psd")).unwrap();

        // Rule resolution.
        assert_eq!(OutputRule::default().resolve(c), None, "same folder, no suffix: overwrite");
        let suffix = OutputRule { suffix: "_clean".into(), ..Default::default() };
        assert_eq!(suffix.resolve(c), Some(d.join("in").join("deep").join("c_clean.psd")));
        let flat = OutputRule { folder: Some(d.join("out")), ..Default::default() };
        assert_eq!(flat.resolve(c), Some(d.join("out").join("c.psd")));
        let tree_rule = OutputRule { folder: Some(d.join("out")), keep_structure: true, ..Default::default() };
        assert_eq!(tree_rule.resolve(c), Some(d.join("out").join("deep").join("c.psd")));

        // New file: input untouched, output clean; already clean files are copied.
        for t in [c, clean] {
            let r = run(t, Plan { output: tree_rule.resolve(t), ..Default::default() });
            assert!(matches!(r.status, Status::Cleaned | Status::AlreadyClean), "{:?}", r.error);
        }
        assert_eq!(fs::read(&c.path).unwrap(), input);
        assert_eq!(ancestors(&d.join("out").join("deep").join("c.psd")), 0);
        assert!(d.join("out").join("clean.psd").exists());

        // New file + move the original to the backup folder.
        let r = run(c, Plan { output: suffix.resolve(c), original: Original::BackupFolder, ..Default::default() });
        assert_eq!(r.status, Status::Cleaned, "{:?}", r.error);
        assert!(!c.path.exists());
        assert_eq!(fs::read(d.join("in").join("deep").join(BACKUP_DIR).join("c.psd")).unwrap(), input);
        assert_eq!(ancestors(&d.join("in").join("deep").join("c_clean.psd")), 0);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn non_psd_file_fails_cleanly() {
        let d = tmpdir("bad");
        fs::write(d.join("x.psd"), b"not a psd at all").unwrap();
        let r = run(&scan(&[d.join("x.psd")])[0], Plan::default());
        assert_eq!(r.status, Status::Failed);
        assert_eq!(fs::read(d.join("x.psd")).unwrap(), b"not a psd at all");
        let _ = fs::remove_dir_all(&d);
    }
}
