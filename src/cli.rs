//! Command-line grammar. This file IS the CLI contract.
//!
//! Rules for editing: the set of commands, flags, value names, defaults and help strings below is
//! the public interface. Do not add, remove or rename anything without updating README.md,
//! tests/cli_help.rs and CHANGELOG.md in the same commit.

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "xe-gmi",
    version,
    about = "Query, monitor, control and persist settings of Intel GPUs on the xe kernel driver",
    long_about = "xe-gmi reads and writes the xe driver's sysfs interfaces directly (no daemon, no \
                  libraries). Read-only commands work as any user. Control commands (set, reset, \
                  persist) need the same root privilege the sysfs files carry; run them under sudo.\n\n\
                  With no command, `xe-gmi` prints the status table.",
    propagate_version = true,
    disable_help_subcommand = true,
    max_term_width = 100
)]
pub struct Cli {
    /// Select one device: index (0, 1, ...), PCI address (0000:e3:00.0) or short form (e3:00.0).
    /// Default: every xe device for read commands; required for control commands when more than
    /// one device is present.
    #[arg(short = 'i', long = "device", global = true, value_name = "SEL")]
    pub device: Option<String>,

    /// Print machine-readable JSON (schema version 1) instead of text.
    #[arg(long, global = true)]
    pub json: bool,

    /// Explain every N/A (which file was looked for, why it was unusable) and show probe details.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Repeat the output every SECS seconds (status, query, processes). Ctrl-C stops.
    #[arg(short = 'u', long = "update", global = true, value_name = "SECS")]
    pub update: Option<f64>,

    /// With --update: stop after N iterations.
    #[arg(long, global = true, value_name = "N")]
    pub count: Option<u64>,

    /// Sampling window in milliseconds for rate-based fields (power draw, utilization). Min 100.
    #[arg(long, global = true, value_name = "MS", default_value_t = 1000)]
    pub sample_ms: u64,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// One-screen summary table of every xe device (the default command).
    Status,

    /// Full per-device report: identity, PCIe, thermal, power, memory, clocks, throttle, engines.
    Info {
        /// Restrict the report to the given section(s). Comma-separated.
        #[arg(long, value_enum, value_delimiter = ',', value_name = "SECTION")]
        section: Vec<Section>,
    },

    /// One line per device: index, PCI address, name, DRM nodes.
    List,

    /// List every query field with units, description and availability on this system.
    Fields,

    /// Print selected fields as CSV (one row per device), or JSON with --json.
    Query {
        /// Comma-separated field names (see `xe-gmi fields`). GT-specific fields use gt<N>.<field>.
        #[arg(long, value_delimiter = ',', value_name = "FIELD,...", required = true)]
        fields: Vec<String>,

        /// Omit the CSV header row.
        #[arg(long)]
        no_header: bool,

        /// Omit the "[unit]" suffix from CSV header names.
        #[arg(long)]
        no_units: bool,
    },

    /// Per-process table: engine utilization and resident VRAM for every visible DRM client.
    Processes {
        /// Sort order.
        #[arg(long, value_enum, default_value_t = ProcSort::Vram)]
        sort: ProcSort,

        /// Aggregate rows per pid (the default), client, cgroup or user.
        #[arg(long, value_enum, default_value_t = GroupBy::Pid)]
        group_by: GroupBy,
    },

    /// Show a control setting with its allowed range and recorded boot default.
    Get {
        #[command(subcommand)]
        what: GetWhat,
    },

    /// Change a control setting (root). Values are written, read back, and the effective value is
    /// reported; values above the firmware maximum are clamped by the driver and reported as such.
    Set {
        #[command(subcommand)]
        what: SetWhat,
    },

    /// Restore a control setting to its boot default (root).
    Reset {
        #[arg(value_enum)]
        what: ResetWhat,

        /// Do not update the persistence state file even if persistence is installed.
        #[arg(long)]
        no_persist: bool,
    },

    /// Manage boot persistence of control settings (udev rule + /etc/xe-gmi/persist.conf).
    Persist {
        #[command(subcommand)]
        action: PersistAction,
    },

    /// Diagnose kernel, driver and device state. Exits 3 when no xe device is usable.
    Doctor {
        /// Write a redacted diagnostic bundle (a directory of text/JSON reports).
        #[arg(long, value_name = "DIR")]
        bundle: Option<PathBuf>,

        /// Keep process names and cgroup paths in the bundle (default: redacted).
        #[arg(long)]
        no_redact: bool,
    },

    /// PCI/NUMA placement of every xe device and, with several devices, their affinity matrix.
    Topology {
        /// Engine, GT and execution-unit inventory from the kernel's device queries.
        #[arg(long)]
        hardware: bool,
    },

    /// PCIe link state and AER error statistics (endpoint and root port).
    Pcie,

    /// Live kernel uevent stream for xe devices (root without a replay fixture).
    Events,

    /// Recover an unresponsive device: save crash dump, unbind, reset, rebind, re-apply persistence.
    Recover {
        /// Print the plan and stop (no writes).
        #[arg(long)]
        dry_run: bool,
        /// Override the soft preconditions (DRM clients, connectors, pending crash save).
        #[arg(long)]
        force: bool,
        /// Reset method; default flr if advertised, else rebind.
        #[arg(long, value_enum)]
        method: Option<RecoverMethod>,
        /// Leave a pending crash dump in place (counts as a soft precondition).
        #[arg(long)]
        no_save_crash: bool,
    },

    /// RAS error counters (DRM RAS netlink, kernel 7.2+); --clear resets them (root).
    Ras {
        #[arg(long)]
        clear: bool,
    },

    /// Kernel crash dumps (devcoredump) for xe devices.
    Crash {
        #[command(subcommand)]
        action: CrashAction,
    },

    /// GuC and HuC firmware versions from the kernel's device query.
    Firmware,

    /// SR-IOV state: PF, enabled VFs and provisioning profiles.
    Sriov {
        #[command(subcommand)]
        action: SriovAction,
    },

    /// GPU memory accounting per cgroup (cgroup v2 dmem controller).
    Cgroups,

    /// One cgroup's GPU memory accounting.
    Cgroup {
        #[command(subcommand)]
        action: CgroupAction,
    },

    /// Per-process monitor: `processes` refreshing every second by default.
    Pmon {
        #[arg(long, value_enum, default_value_t = GroupBy::Pid)]
        group_by: GroupBy,

        /// Keep clients that disappeared visible for SECS seconds, marked "exited".
        #[arg(long, value_name = "SECS", default_value_t = 0)]
        show_exited: u64,
    },

    /// Print a shell completion script to stdout.
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Write the man page (xe-gmi.1) into a directory.
    #[command(hide = true)]
    Man {
        /// Output directory.
        #[arg(long, value_name = "DIR", default_value = ".")]
        out: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
pub enum GetWhat {
    /// Effective power limit, every visible limit (PL1/PL2, card/pkg), windows, critical limit.
    PowerLimit,
    /// Per-GT min/max, hardware range (rpn..rp0), efficient and boot-default frequencies.
    Clocks,
    /// Per-GT firmware power profile.
    PowerProfile,
}

#[derive(Subcommand, Debug)]
pub enum SetWhat {
    /// Set the power limit. VALUE accepts watts by default: 150, 150W, 150.5W, 150000mW, 150000000uW.
    PowerLimit {
        /// The new limit.
        value: String,

        /// Which limit to write when both exist. auto = PL1 (sustained) if present, else PL2 (burst).
        #[arg(long, value_enum, default_value_t = LimitKind::Auto)]
        limit: LimitKind,

        /// Which hwmon channel: card (power1_*) or pkg (power2_*).
        #[arg(long, value_enum, default_value_t = Channel::Card)]
        channel: Channel,

        /// Do not record the value in the persistence state file even if persistence is installed.
        #[arg(long, conflicts_with = "persist")]
        no_persist: bool,

        /// Record the value for reapplication at boot; errors if persistence is not installed.
        #[arg(long)]
        persist: bool,
    },

    /// Set the power-limit averaging window. VALUE accepts ms (default), s: 15, 15ms, 28s.
    PowerWindow {
        /// The new window.
        value: String,

        /// Which limit's window to write. auto = PL1 (sustained) if present, else PL2 (burst).
        #[arg(long, value_enum, default_value_t = LimitKind::Auto)]
        limit: LimitKind,

        /// Which power channel (card = power1, pkg = power2).
        #[arg(long, value_enum, default_value_t = Channel::Card)]
        channel: Channel,

        /// Do not record the value in the persistence state file even if persistence is installed.
        #[arg(long, conflicts_with = "persist")]
        no_persist: bool,

        /// Record the value for reapplication at boot; errors if persistence is not installed.
        #[arg(long)]
        persist: bool,
    },

    /// Set GT frequency bounds in MHz. At least one of --min/--max is required. Applies to every GT
    /// of the device unless --gt is given.
    Clocks {
        /// Minimum frequency in MHz (must be >= rpn_freq and <= max).
        #[arg(long, value_name = "MHZ")]
        min: Option<u32>,

        /// Maximum frequency in MHz (must be <= rp0_freq and >= min).
        #[arg(long, value_name = "MHZ")]
        max: Option<u32>,

        /// Only this GT (global GT id as in tile*/gt<N>).
        #[arg(long, value_name = "N")]
        gt: Option<u32>,

        /// Do not record the value in the persistence state file even if persistence is installed.
        #[arg(long, conflicts_with = "persist")]
        no_persist: bool,

        /// Record the value for reapplication at boot; errors if persistence is not installed.
        #[arg(long)]
        persist: bool,
    },

    /// Select the firmware power profile (kernel 6.18+).
    PowerProfile {
        #[arg(value_enum)]
        profile: Profile,

        /// Only this GT (global GT id as in tile*/gt<N>).
        #[arg(long, value_name = "N")]
        gt: Option<u32>,

        /// Do not record the value in the persistence state file even if persistence is installed.
        #[arg(long, conflicts_with = "persist")]
        no_persist: bool,

        /// Record the value for reapplication at boot; errors if persistence is not installed.
        #[arg(long)]
        persist: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum SriovAction {
    /// Show PF state and per-VF provisioning.
    Status,

    /// Enable N VFs (writes sriov_numvfs; requires N <= sriov_totalvfs and none enabled).
    Enable {
        /// Number of VFs to enable.
        n: u32,
    },

    /// Disable all VFs (writes sriov_numvfs=0; fails while VFs are in use).
    Disable,

    /// Per-VF administration.
    Vf {
        /// VF index (1..=sriov_totalvfs) or `all` for the bulk profile.
        target: String,
        #[command(subcommand)]
        action: VfAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum VfAction {
    /// Provision a VF (or the bulk profile); never persisted.
    Set {
        #[arg(long, value_name = "SIZE")]
        vram: Option<String>,
        #[arg(long, value_name = "TIME")]
        quantum: Option<String>,
        #[arg(long, value_name = "TIME")]
        timeout: Option<String>,
        #[arg(long, value_enum)]
        priority: Option<Priority>,
    },
    /// Stop a VF's context (interrupts its user; requires --force).
    Stop {
        #[arg(long)]
        force: bool,
    },
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum Priority {
    Low,
    Normal,
    High,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum RecoverMethod {
    Flr,
    Bus,
    Rebind,
}

#[derive(Subcommand, Debug)]
pub enum CgroupAction {
    /// Show one cgroup's GPU memory usage, limits and attributed clients.
    Show { path: String },

    /// Set the device-memory limit of one cgroup (binary units or `max`; not persisted).
    Set {
        path: String,
        #[arg(long, value_name = "SIZE|max")]
        vram_max: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum CrashAction {
    /// List pending crash dumps of xe devices.
    List,
    /// Print a crash dump to stdout.
    Show { id: u32 },
    /// Write a crash dump to a file.
    Save {
        id: u32,
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
    },
    /// Release a crash dump (frees the kernel's copy; the next hang can be captured).
    Release { id: u32 },
}

#[derive(Subcommand, Debug)]
pub enum PersistAction {
    /// Install the udev rule and create the state file (root). Idempotent; prints the rule.
    Install {
        /// Absolute path of the xe-gmi binary the udev rule should run. Must live outside home
        /// directories. Default: this executable if it is under /usr or /opt, otherwise an error.
        #[arg(long, value_name = "PATH")]
        exe: Option<PathBuf>,
    },
    /// Remove the udev rule (root). The state file is kept unless --purge is given.
    Remove {
        /// Also delete /etc/xe-gmi/persist.conf.
        #[arg(long)]
        purge: bool,
    },
    /// Show the rule, the state file, and the result of the last apply.
    Show,
    /// Apply the state file now (root). Run by udev at driver bind; usable by hand.
    Apply {
        /// Devpath passed by udev (%p); restricts the apply to that PCI device.
        #[arg(long, value_name = "DEVPATH")]
        udev_devpath: Option<String>,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Driver,
    Device,
    Pcie,
    Thermal,
    Power,
    Memory,
    Clocks,
    Throttle,
    Engines,
    Processes,
    Topology,
    Connectors,
    Sriov,
    Crash,
    Firmware,
    Hardware,
    All,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupBy {
    Pid,
    Client,
    Cgroup,
    User,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcSort {
    Vram,
    Util,
    Pid,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitKind {
    Auto,
    Pl1,
    Pl2,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Card,
    Pkg,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Base,
    #[value(name = "power-saving", alias = "power_saving")]
    PowerSaving,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetWhat {
    PowerLimit,
    Clocks,
    PowerProfile,
    All,
}
