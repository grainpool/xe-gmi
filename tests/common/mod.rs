//! Shared harness for the black-box CLI tests (0.1.2+). Every test runs the real binary against a
//! fixture tree via the environment seams. Replaces the 0.1.x file: identical helpers plus the
//! cgroup, kmsg and kabi-replay seams.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

pub const NOW: &str = "1789381351";

pub fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
pub fn fixture(name: &str) -> PathBuf {
    manifest().join("fixtures/synthetic").join(name)
}
pub fn pciids() -> PathBuf {
    manifest().join("fixtures/pci.ids.mini")
}

pub struct TempTree {
    pub root: PathBuf,
}
impl TempTree {
    pub fn tree(&self) -> PathBuf {
        self.root.join("tree")
    }
    pub fn sys(&self) -> PathBuf {
        self.root.join("tree/sys")
    }
    pub fn proc_(&self) -> PathBuf {
        self.root.join("tree/proc")
    }
    pub fn etc(&self) -> PathBuf {
        self.root.join("etc")
    }
    pub fn state(&self) -> PathBuf {
        self.root.join("state")
    }
    /// Device directory: resolves through sys/bus/pci/devices so deep (bridge) layouts work too.
    pub fn device(&self, pci: &str) -> PathBuf {
        fs::canonicalize(self.sys().join("bus/pci/devices").join(pci)).expect("device dir")
    }
    pub fn read(&self, rel_to_device: &str, pci: &str) -> String {
        fs::read_to_string(self.device(pci).join(rel_to_device))
            .unwrap_or_else(|e| panic!("read {rel_to_device}: {e}"))
            .trim_end()
            .to_string()
    }
    pub fn read_abs(&self, rel_to_tree: &str) -> String {
        fs::read_to_string(self.tree().join(rel_to_tree))
            .unwrap_or_else(|e| panic!("read {rel_to_tree}: {e}"))
            .trim_end()
            .to_string()
    }
    pub fn readback(&self, rel_to_device: &str, pci: &str, value: &str) {
        fs::write(
            self.device(pci).join(format!("{rel_to_device}.readback")),
            format!("{value}\n"),
        )
        .unwrap();
    }
    pub fn as_uid(&self, uid: u32) {
        let p = self.proc_().join("self/status");
        fs::write(
            &p,
            format!(
                "Name:\txe-gmi\nState:\tR (running)\nPid:\t1\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n"
            ),
        )
        .unwrap();
    }
}
impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn writable_copy(fixture_name: &str) -> TempTree {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!(
        "xe-gmi-test-{}-{}-{}",
        std::process::id(),
        n,
        fixture_name
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("etc")).unwrap();
    fs::create_dir_all(root.join("state")).unwrap();
    let st = Command::new("cp")
        .arg("-a")
        .arg(fixture(fixture_name))
        .arg(root.join("tree"))
        .status()
        .expect("cp -a");
    assert!(st.success(), "cp -a failed");
    TempTree { root }
}

pub struct Runner {
    pub tree: PathBuf,
    pub sys: PathBuf,
    pub proc_: PathBuf,
    pub modules: PathBuf,
    pub etc: PathBuf,
    pub state: PathBuf,
    pub t1: Option<(PathBuf, PathBuf)>,
    pub kabi: bool,
    pub extra: Vec<(String, String)>,
}

impl Runner {
    pub fn fixture(name: &str) -> Runner {
        let root = fixture(name);
        let t1 = fixture(&format!("{name}-t1"));
        Runner {
            tree: root.clone(),
            sys: root.join("sys"),
            proc_: root.join("proc"),
            modules: root.join("lib/modules"),
            etc: root.join("etc"),
            state: std::env::temp_dir().join(format!("xe-gmi-ro-state-{}", std::process::id())),
            t1: if t1.exists() {
                Some((t1.join("sys"), t1.join("proc")))
            } else {
                None
            },
            kabi: false,
            extra: vec![],
        }
    }
    pub fn temp(t: &TempTree, fixture_name: &str) -> Runner {
        let t1 = fixture(&format!("{fixture_name}-t1"));
        Runner {
            tree: t.tree(),
            sys: t.sys(),
            proc_: t.proc_(),
            modules: t.root.join("tree/lib/modules"),
            etc: t.etc(),
            state: t.state(),
            t1: if t1.exists() {
                Some((t1.join("sys"), t1.join("proc")))
            } else {
                None
            },
            kabi: false,
            extra: vec![],
        }
    }
    /// Enable the kabi replay seam (`<tree>/kabi/`): DRM queries, RAS and uevents come from files.
    pub fn kabi(mut self) -> Runner {
        self.kabi = true;
        self
    }
    pub fn env(mut self, k: &str, v: &str) -> Runner {
        self.extra.push((k.to_string(), v.to_string()));
        self
    }
    pub fn run(&self, args: &[&str]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_xe-gmi"));
        cmd.env_clear();
        cmd.env("PATH", "/usr/bin:/bin");
        cmd.env("XE_GMI_SYSFS_ROOT", &self.sys);
        cmd.env("XE_GMI_PROCFS_ROOT", &self.proc_);
        cmd.env("XE_GMI_MODULES_ROOT", &self.modules);
        cmd.env("XE_GMI_ETC_ROOT", &self.etc);
        cmd.env("XE_GMI_STATE_ROOT", &self.state);
        cmd.env("XE_GMI_PCIIDS", pciids());
        cmd.env("XE_GMI_NOW", NOW);
        cmd.env("XE_GMI_UDEVADM", "");
        cmd.env("XE_GMI_EXE_PATH", "/usr/local/bin/xe-gmi");
        cmd.env("XE_GMI_CGROUP_ROOT", self.sys.join("fs/cgroup"));
        let kmsg = self.tree.join("kmsg.txt");
        cmd.env(
            "XE_GMI_KMSG",
            if kmsg.exists() {
                kmsg
            } else {
                PathBuf::from("")
            },
        );
        if self.kabi {
            cmd.env("XE_GMI_KABI_REPLAY", self.tree.join("kabi"));
        }
        if let Some((s, p)) = &self.t1 {
            cmd.env("XE_GMI_SYSFS_ROOT_T1", s);
            cmd.env("XE_GMI_PROCFS_ROOT_T1", p);
            cmd.env("XE_GMI_FIXTURE_DT_MS", "2000");
        }
        for (k, v) in &self.extra {
            cmd.env(k, v);
        }
        cmd.arg("--sample-ms").arg("100");
        cmd.args(args);
        cmd.output().expect("failed to spawn xe-gmi")
    }
}

pub fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
pub fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}
pub fn code(o: &Output) -> i32 {
    o.status.code().unwrap_or(-1)
}
pub fn golden_path(name: &str) -> PathBuf {
    manifest().join("tests/golden").join(name)
}
pub fn assert_golden(actual: &str, name: &str) {
    let path = golden_path(name);
    if std::env::var("XE_GMI_UPDATE_GOLDEN").is_ok() {
        fs::write(&path, actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing golden {}: {e}", path.display()));
    if actual != expected {
        let mut report = String::new();
        for (i, (a, e)) in actual.lines().zip(expected.lines()).enumerate() {
            if a != e {
                report.push_str(&format!(
                    "line {}:\n  expected: {e:?}\n  actual:   {a:?}\n",
                    i + 1
                ));
                break;
            }
        }
        if actual.lines().count() != expected.lines().count() {
            report.push_str(&format!(
                "line count: expected {} actual {}\n",
                expected.lines().count(),
                actual.lines().count()
            ));
        }
        panic!("golden mismatch for {name}:\n{report}\n--- actual ---\n{actual}\n--- expected ---\n{expected}");
    }
}
pub fn is_root() -> bool {
    fs::read_to_string("/proc/self/status")
        .map(|s| {
            s.lines()
                .any(|l| l.starts_with("Uid:") && l.split_whitespace().nth(1) == Some("0"))
        })
        .unwrap_or(false)
}
pub fn assert_contains(hay: &str, needle: &str) {
    assert!(
        hay.contains(needle),
        "expected to find {needle:?} in:\n{hay}"
    );
}
pub fn assert_not_contains(hay: &str, needle: &str) {
    assert!(
        !hay.contains(needle),
        "did not expect {needle:?} in:\n{hay}"
    );
}
pub fn exists(p: &Path) -> bool {
    p.exists()
}
