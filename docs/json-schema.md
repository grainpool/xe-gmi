# JSON output

`--json` is a global flag (works with every command): the whole output becomes **one JSON object**
with `"schema_version": 1` on stdout. The normative schema is
[`schema/xe-gmi-v1.schema.json`](../schema/xe-gmi-v1.schema.json); key order in the emitted text is
the order of the schema's `properties`. 2-space indent, `": "` after keys, one array element per
line, numbers printed with the same decimals as the text output (`12.40`, `5.0`), `null` for every
N/A, strings JSON-escaped.

```
$ xe-gmi --json query --fields temp.pkg,power.draw
{
  "schema_version": 1,
  "generated_at": "2026-09-14T10:22:31Z",
  "fields": ["temp.pkg", "power.draw"],
  "rows": [
    { "temp.pkg": 41.0, "power.draw": 12.40 }
  ]
}
```

`rows[]` has one object per selected device. (`power.draw` is `12.40` exactly as the text table
prints it.)

## Top-level objects per command

| Command | Object |
|---|---|
| `status`, `info`, `list` | `{schema_version, tool{name,version}, generated_at, kernel, driver, devices[], unavailable[]}` |
| `query` | `{schema_version, generated_at, fields[], rows[]}` — one row per device, keys = requested field names |
| `processes` | `{schema_version, generated_at, sample_ms, visible_all_users, processes[{pid, name, pci_address, client_id, resident_vram_bytes, engine_pct{rcs,ccs,vcs,vecs,bcs}}]}` |
| `get …` | `{schema_version, generated_at, devices[{pci_address, …same sub-object as in `status` plus `boot_default`}]}` |
| `set` / `reset` | `{schema_version, generated_at, changes[{pci_address, gt, control, requested, effective, raw_written, raw_readback, clamped}], persistence}` |
| `persist show` | `{schema_version, installed, rule_path, state_path, entries[{pci_address, key, value}], last_apply{timestamp, results[]}}` |
| `fields` | `{schema_version, fields[{name, unit, available, description}]}` |
| `doctor` | `{schema_version, generated_at, kernel, distro, xe_module{state,path}, gpus[{pci_address, vendor_id, device_id, driver, verdict, first_kernel}], usable_xe_devices, capabilities[], result}` |

## Field naming

`devices[]` items in `status`/`info`/`list` use the same names as the query field catalog
(`xe-gmi fields`) grouped into sub-objects: `pci{…, link{…}}`, `thermal[]`, `power{…, limit{…}}`,
`memory{…}`, `gts[{…, clock_mhz{cur,act,min,max,rp0,rpe,rpn,rpa}, throttle{active,reasons[]}}]`,
`fans[]`, `engines{rcs,ccs,vcs,vecs,bcs}`, `processes_count`, `persistence_installed`.
Every value that prints `N/A` in text mode is `null` here, and the `unavailable[]` array
(`{device, field, reason}`) explains every one — it is always populated, not only with `-v`.

## Versioning promise

Within `schema_version: 1` keys are only **added** — never renamed, removed, or retyped. Consumers
must ignore unknown keys. A breaking change raises `schema_version`; the schema file for the current
version stays published as `xe-gmi-v1.schema.json`.
