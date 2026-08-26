# Governance and modding

**Status:** Proposed baseline reflecting confirmed authority

## Verse DAO executor

Initial executor:

- Safe: `0x4E7cf530B84DAE10c4500737C3408761a9385051`
- Ethereum mainnet.
- Three owners.
- Two-signature threshold.

Equivalent-address Safe deployments are planned for Sepolia, Base, and Base Sepolia.

The Safe may control:

- Treasury.
- Official AMM deployment and initial capital-market liquidity.
- Approved contract upgrades.
- Chain registry.
- Official content manifests.
- Security Council delegation.
- Emergency pause and unpause.
- Protocol configuration within published bounds.

## Open Metaverse governing framework

The Verse adopts the Open Metaverse as its contractual governing framework using the meaning in Section 13 of the Bittrees Bounties Terms of Use effective 2026-08-12. The canonical project interpretation is documented in [Open Metaverse governing framework](open-metaverse-framework.md).

The framework guides consent, property, voluntary contracts, transparent rules, code-backed evidence, interoperability, decentralization, portable reputation, and nonaggression. It is not represented as a sovereign or territorial jurisdiction and does not displace applicable nonwaivable rights or law.

## Governance process

Normal changes should follow:

1. Public proposal.
2. Technical and economic impact statement.
3. Security review.
4. Test deployment or staging manifest.
5. Public comment.
6. Required approval.
7. Timelocked execution where appropriate.
8. Published deployment or content hash.

The initial Safe is an executor, not by itself a complete community-voting system. A broader governance mechanism can be introduced later without replacing the Safe immediately.

## Security Council

The Verse DAO may delegate narrowly scoped authority to a Security Council.

Permitted emergency reasons:

- Unlimited-resource bug.
- Asset duplication.
- Custody insolvency.
- Incorrect receipt supply.
- Compromised bridge or relayer.
- Malicious official mod.
- Account or recovery vulnerability.
- Critical remote execution or server-authority bypass.

Not permitted:

- Legitimate price volatility.
- Large valid sales or deposits.
- Disagreement with a lawful market strategy.
- Protecting a favored trader from loss.
- Routine content balance that does not threaten integrity.

A pause remains in effect until the Safe or authorized council unpauses it. There is no automatic expiration.

Public emergency record:

- Incident ID.
- Reason and evidence.
- Affected scope.
- Approving authority.
- Time.
- User impact.
- Remediation.
- Unpause decision.
- Post-incident report.

## Company DAOs

A company has:

- Canonical company ID.
- Public profile.
- Members.
- Flexible ranks.
- Game permissions.
- Treasury.
- Assets.
- Contracts.
- Markets.
- Governance configuration.

Supported governance patterns may include:

- Founder authority.
- Multisig council.
- One-member-one-vote.
- Token voting.
- Reputation voting.
- Departmental mandates.
- Hybrid constitutions.

Company governance cannot override server conservation, protected zones, official mod policy, or protocol security boundaries.

## Official content manifests

The official universe loads a signed manifest defining:

- Blocks.
- Resources.
- Recipes.
- Machines.
- Components.
- Scripts.
- Visual assets.
- Balance values.
- Dependencies.
- Schema versions.
- Licenses.
- Content hashes.

A server and client must agree on the manifest range before gameplay.

## Mod submission

```text
Proposal
→ provenance and license validation
→ static security checks
→ sandbox execution
→ performance budget
→ conservation/economy simulation
→ staging universe
→ public review
→ DAO approval
→ signed manifest
→ phased activation
```

## Mod sandbox

Early mods should be limited to:

- Declarative definitions.
- Approved visual and audio assets.
- Sandboxed scripts.
- Versioned APIs.
- CPU, memory, storage, and event budgets.
- No filesystem, network, process, key, or unrestricted reflection access.

Native server plugins are deferred until a security model exists.

## Malicious or defective mods

The Security Council may disable an official manifest or affected feature. Existing assets require an explicit migration:

- Freeze affected production.
- Preserve ownership.
- Quarantine unsafe behavior.
- Convert, refund, deprecate, or destroy only under published rules.
- Record every administrative asset change.

## Private servers

Private servers may run any lawful content their operators choose.

They must use:

- Separate universe ID.
- Separate issuer namespace.
- Separate identity/configuration boundary.
- Non-canonical asset IDs.
- No official deposit or withdrawal bridge.
- No ability to satisfy official contracts.
- No canonical lifecycle roots.

A private server may integrate BIT independently, but The Verse does not certify or import its game assets.

## User-generated content

Blueprints, skins, clothing, avatar elements, and other UGC require:

- Rights attestation.
- Declared license.
- Authorship/provenance.
- Content safety review.
- Technical/performance validation.
- Royalty rules.
- Versioning.
- Takedown and dispute process.

The visual language must remain original and may not reproduce protected franchise content.
