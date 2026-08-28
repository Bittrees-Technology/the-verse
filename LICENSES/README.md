# Licensing map

This repository uses licenses by component.

| Component | License |
| --- | --- |
| Game client | AGPL-3.0-or-later |
| Authoritative server and first-party services | AGPL-3.0-or-later |
| Smart contracts | AGPL-3.0-or-later unless noted otherwise |
| Public SDKs and schemas | Apache-2.0 |
| Reusable source art and media | CC BY-SA 4.0 unless noted otherwise |
| The Verse name and official identity | Reserved pending trademark policy |

The root [LICENSE](../LICENSE) contains the GNU Affero General Public License v3. Apache-licensed SDK directories must include the Apache license and a clear SPDX identifier. Asset directories must include authorship, source, and license metadata.

Third-party dependencies and assets must be added to a software or content bill of materials before release.

The repository also carries verbatim third-party license texts when a shipped
runtime dependency requires a license not already represented by the component
licenses. Godot-rust's Mozilla Public License 2.0 is included in
[`MPL-2.0.txt`](MPL-2.0.txt).

The current P1.5 dependency record is in [THIRD_PARTY.md](THIRD_PARTY.md). `Cargo.lock` is the canonical version lock for the Rust dependency graph.
