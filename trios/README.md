# GOLDEN CHAIN — φ-Structured Silicon Network

## From GOLDEN BRIDGE to GOLDEN CHAIN

**BRIDGE** is static — it connects two fixed points. One-time proof. Cannot mine.
**CHAIN** is alive — it grows link by link, block by block. Continuous proof. Mineable.

Every link in the Golden Chain is a silicon witness of the algebraic identity: φ² + φ⁻² = 3 = TRINITY

---

# Trinity S³AI — H4 Coxeter Group / Standard Model Unification (Research Foundation)

[![CI](https://github.com/gHashTag/trinity-s3ai/actions/workflows/ci.yml/badge.svg)](https://github.com/gHashTag/trinity-s3ai/actions/workflows/ci.yml)
[![Anti-Numerology Gate](https://img.shields.io/badge/anti--numerology-PASS-brightgreen)](https://github.com/gHashTag/trinity-s3ai/actions/workflows/ci.yml)
[![Coq](https://img.shields.io/badge/Coq-8.20.1-blue)](https://coq.inria.fr)
[![Active Research](https://img.shields.io/badge/active--research-blue)](docs/TECH_TREE.md)
[![DOI](https://img.shields.io/badge/DOI-pending%20Zenodo-lightgrey)](scripts/prepare_zenodo.md)

---


## English

### Status

> **This is an active boundary-mapping research program. See [RESEARCH_STATUS.md](RESEARCH_STATUS.md).**

This repository documents an ongoing investigation into whether geometric invariants of the H4 Coxeter group (and related structures such as the 600-cell and Clifford algebras Cl(8)) can encode or constrain the parameters of the Standard Model of particle physics.

**Boundary theorems (BT-1–BT-4)** map specific obstruction points: they prove that *certain direct constructions* from H4 geometry do not reproduce the SM. These are **guideposts**, not tombstones — they narrow the search space and direct future exploration toward Tracks A (honest phenomenology), B (Cl(8) / J₃(𝕆)), and C (publication of boundary results).

The project maintains a living catalog of 59 numerical coincidences between H4 invariants and PDG 2024 measurements, all formally verified in Coq and tagged by epistemic status. Whether these coincidences are deep or accidental is itself an open research question.

### Why $TRI Can Only Be Mined on Our Chips

Each Golden Chain link requires a **φ-structured arithmetic unit** — hardware implementing:

1. **Galois Field GF(16)** — GF(2⁴) with primitive polynomial x⁴ + x + 1
2. **Lucas Chain L₂ = 3** — recursive link
3. **0.694-bit reduction** per step — φ⁻¹ ≈ 0.618 quantization
4. **0x47C0 anchor** — silicon reset witness via dot4 trace

*No generic CPU implements this. Only our chips (TinyTapeout TTSKY26b, FPGA, future ASIC) have the φ-arithmetic in silicon.*

---

### Quick Links

| Resource | Description |
|----------|-------------|
| [docs/REVIEW_GUIDE.md](docs/REVIEW_GUIDE.md) | **For reviewers:** 10-minute path with commands and expected outputs |
| [docs/REPOSITORY_MAP.md](docs/REPOSITORY_MAP.md) | Where every kind of artifact lives in this repository |
| [docs/CLAIM_STATUS.md](docs/CLAIM_STATUS.md) | Claim-status rule book (verified / empirical fit / open / refuted) |
| [docs/TECH_TREE.md](docs/TECH_TREE.md) | Layered status of the whole stack (infra → proofs → fits → game → paper) |
| [docs/REVIEW_CHECKLIST.md](docs/REVIEW_CHECKLIST.md) | Mechanical review-readiness checklist |
| [RESEARCH_STATUS.md](RESEARCH_STATUS.md) | Boundary map: which hypotheses are open, obstructed, or under exploration |
| [EPISTEMOLOGY.md](EPISTEMOLOGY.md) | Calculation-primacy doctrine: how model-dependent extractions are handled |
| [PREDICTIONS_PREREGISTERED.md](PREDICTIONS_PREREGISTERED.md) | 9 pre-registered predictions (0 theoretically confirmed, 4 numerically verified, 4 open, 1 withdrawn) |
| [ROADMAP_WAVE17_PLUS.md](ROADMAP_WAVE17_PLUS.md) | **Active research program**: Tracks A (honest phenomenology), B (Cl(8) / J3(O)), C (boundary-result paper) |
| [paper/CHANGELOG_v1_to_v2.md](paper/CHANGELOG_v1_to_v2.md) | Wave 17 paper v2 changelog — new obstruction BT-7, updated counters, E8-plumbing / string-correspondence boundary notes |
| [SECURITY.md](SECURITY.md) | Security policy and reporting path |
| [HONESTY_MANIFEST.md](HONESTY_MANIFEST.md) | **Ground-truth statistics** (comments stripped) |
| [BoundaryTheorems.v](proofs/trinity/BoundaryTheorems.v) | Formal boundary theorems BT-1–BT-4 (boundary markers, not dead ends) |
| [Catalog42.v](proofs/trinity/Catalog42.v) | 42 SM parameter formulas (tagged phenomenological) |
| [admitted_log.md](docs/analysis/admitted_log.md) | Log of all Admitted with their tags |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Contribution rules + build instructions |
| [scripts/prepare_zenodo.md](scripts/prepare_zenodo.md) | Zenodo publication guide |
| [FOUNDATIONS.md](docs/analysis/FOUNDATIONS.md) | Axiom stratification and open assumptions |
| [proofs/clifford_cl8/README.md](proofs/clifford_cl8/README.md) | **Track B (Wave 12)** — Cl(p,q) formalization launch (T1–T3) |
| [GOLDEN CHAIN (live)](https://t27.ai/trinity-s3ai/) | Live hypothesis-discovery puzzle (not evidence; see game README) |

> **For external reviewers:** start with
> [`docs/REVIEW_GUIDE.md`](docs/REVIEW_GUIDE.md). It lists what to read,
> what commands to run, and how to judge each kind of claim against
> [`docs/CLAIM_STATUS.md`](docs/CLAIM_STATUS.md). The live GOLDEN CHAIN
> canvas is at <https://t27.ai/trinity-s3ai/>; CI status is shown by the
> badges at the top of this file. Known blind spots and proof debt are
> tracked in [`docs/TECH_TREE.md`](docs/TECH_TREE.md). **No Theory-of-
> Everything claim and no prize claim is made.**

---

## Claim Ledger (SSOT)

The table below is auto-generated from [`docs/claims.yaml`](docs/claims.yaml)
by [`scripts/generate_claims.py`](scripts/generate_claims.py). The same
ledger drives the GOLDEN CHAIN card data at
[`games/trinity_fold/fixtures/generated_claim_cards.json`](games/trinity_fold/fixtures/generated_claim_cards.json).
Status vocabulary is defined in [`docs/CLAIM_STATUS.md`](docs/CLAIM_STATUS.md).

<!-- CLAIMS_TABLE:START -->

_Generated from [`docs/claims.yaml`](docs/claims.yaml) by [`scripts/generate_claims.py`](scripts/generate_claims.py). Do not edit this block by hand._

| Claim | Layer | Status | Evidence | Blocking gap |
|-------|-------|--------|----------|--------------|
| GOLDEN CHAIN live canvas deployed on GitHub Pages (Rust + wasm) | L0 | `verified` | .github/workflows/pages.yml; live at https://t27.ai/trinity-s3ai/ | — |
| Wave 17 honest counter reports 0 real `Admitted.` in proofs/trinity/ | L2 | `verified` | scripts/count_admitted_honest.py output; HONESTY_MANIFEST.md | Holds only for proofs/trinity/. Track B (proofs/clifford_cl8/) retains load-bearing Axioms with citations; the Lagrangian derivation is not closed. |
| BT-1–BT-4: four formal Boundary theorems closed with `Qed.` | L2 | `verified` | proofs/trinity/BoundaryTheorems.v (BT-1, BT-2, BT-3, BT-4 all Qed) | — |
| E8 plumbing: eta discrepancy does NOT converge to -2 | L4 | `high_risk_or_falsified` | paper/CHANGELOG_v1_to_v2.md (Honesty Notes); Wave 17 audit | — |
| No known string / heterotic / F-theory / orbifold compactification rescues the SM hierarchy from H4 or F4 | L4 | `high_risk_or_falsified` | ROADMAP_WAVE17_PLUS.md (Wave 17.2 Findings) | — |
| `a4` conversion factor not fully reconciled across three derivations | L3 | `open_conjecture` | docs/analysis/a4_conversion_factor_analysis.md; docs/analysis/a4_honest_resolution.md | Produce a single derivation that all three (analytic, spectral, fit) paths agree on, or document why they cannot agree and downgrade the relevant fits. |
| `m_H = 4 phi^3 e^2 ~ 125.1 GeV` is the Higgs mass | L5 | `empirical_fit` | proofs/trinity/HiggsPrediction.v (interval bound); docs/analysis/higgs_potential_proven.md | Derive m_H from H4 / NCG structure rather than from a (phi, e) monomial. Any such derivation must pass the anti-numerology gate and avoid the BT-2 sigma-field obstruction. |
| GOLDEN CHAIN puzzle is a hypothesis-discovery game, not evidence | L6 | `verified` | games/trinity_fold/README.md; ring0_core::ClaimStatus enum | — |
| No Theory-of-Everything claim and no prize claim is made | L1 | `verified` | docs/CLAIM_STATUS.md §2; README.md preamble | — |

<!-- CLAIMS_TABLE:END -->

To update: edit `docs/claims.yaml`, run `python3 scripts/generate_claims.py`,
commit both the YAML and the regenerated artefacts. CI runs the same
script with `--check` and fails if anything is stale.

---

## Hypothesis Verification Tech Tree

**The goal is not to prove everything by prose; the goal is to make each
claim unlock only after the previous verification layer passes.** This
repository is a layered hypothesis-verification stack, not a finished
theory. Lower layers (infrastructure, claim ledger, formal proofs) gate
what is allowed to be claimed at higher layers (geometry, Lagrangian,
fits, game, publication). When a layer is not green, the layers above
it are explicitly flagged as `open_conjecture` or
`high_risk_or_falsified` per [`docs/CLAIM_STATUS.md`](docs/CLAIM_STATUS.md).

The fuller table (status per row, file pointers, next milestone) lives
in [`docs/TECH_TREE.md`](docs/TECH_TREE.md). The diagram below is the
grep-friendly summary.

```
                       +-------------------------------------+
                       | L7  Publication                     |
                       | paper v2 / arXiv / Zenodo / talks   |
                       +-------------------^-----------------+
                                           |
                       +-------------------+-----------------+
                       | L6  GOLDEN CHAIN Game              |
                       | rings, claim cards, falsified floor |
                       +-------------------^-----------------+
                                           |
                       +-------------------+-----------------+
                       | L5  Numerical Fits & Validation     |
                       | 59 formulas, error windows, p-value |
                       +-------------------^-----------------+
                                           |
                       +-------------------+-----------------+
                       | L4  NCG / Lagrangian                |
                       | spectral triple, sigma-field, SM L  |
                       +-------------------^-----------------+
                                           |
                       +-------------------+-----------------+
                       | L3  Geometry  (H4 / 600-cell / Cl)  |
                       | root system, Snub 24-cell, Cl(8)    |
                       +-------------------^-----------------+
                                           |
                       +-------------------+-----------------+
                       | L2  Formal Proofs (Coq / Lean)      |
                       | BT-1..BT-4 Qed, refutation theorems |
                       +-------------------^-----------------+
                                           |
                       +-------------------+-----------------+
                       | L1  Claim Ledger                    |
                       | 5-status vocabulary, no-overclaim   |
                       +-------------------^-----------------+
                                           |
                       +-------------------+-----------------+
                       | L0  Infrastructure                  |
                       | CI, anti-numerology gate, Pages,    |
                       | honest counter, Cargo + Coq builds  |
                       +-------------------------------------+
```

Each layer below states what is currently **Checked**, what is a known
**Blind spot**, and what the **Next unlock** is. Status keywords use
[`docs/CLAIM_STATUS.md`](docs/CLAIM_STATUS.md).

### L0 - Infrastructure
- **Checked:** Rust workspace tests (`trinity_rust/`, `games/trinity_fold/`),
  Pages / wasm deploy of the GOLDEN CHAIN canvas, anti-numerology gate,
  Coq build gate, Lean scaffolding gate, honest counter, live game at
  <https://t27.ai/trinity-s3ai/>.
- **Blind spots:** the gates verify build / tag hygiene, not physics; CI
  is heavy; no single dashboard yet shows proof-debt and claim-counts in
  one place.
- **Next unlock:** CI Observatory dashboard, proof-debt counter wired
  into PRs, Claim Gate (every PR touching public docs must point to a
  ledger row), Artifact Gate for the arXiv bundle.

### L1 - Claim Ledger
- **Checked:** five-status vocabulary
  (`verified` / `empirical_fit` / `open_conjecture` / `high_risk_or_falsified` /
  `unverified`), no Theory-of-Everything wording, no prize wording, the
  game is framed as a hypothesis puzzle and not as evidence.
- **Blind spots:** older `*_v44.md` / `*_v46.md` / `IMPACT_*` documents
  are not all linked to ledger rows yet.
- **Next unlock:** single source of truth (`claims.yaml` / `claims.json`)
  consumed by README, game tile catalogue, and docs generators, so every
  public claim is traceable to one row.

### L2 - Formal Proofs
- **Checked:** Coq / Rocq build (Wave 17 baseline: 1 790 `Qed.`+`Defined.`
  across the canonical tree; 0 real `Admitted.` after comment stripping),
  Lean 4 scaffolding (`TrinityLean/`), 14 refutation theorems
  (`*_refuted`), BT-1..BT-4 closed with `Qed.`.
- **Blind spots:** the formal layer does **not** close the Lagrangian
  derivation; Track B Clifford statements still rest on cited `Axiom`s;
  Lean side is red on `main` (`H4RootSystem.lean`, `Snub24Z3.lean`).
- **Next unlock:** machine-readable proof map (which `Qed.` discharges
  which claim row), no floating theorem rule, expanded refutation
  library, bridge theorems between geometry and the spectral side.

### L3 - Geometry (H4 / 600-cell / Clifford)
- **Checked:** H4 root system (order 14 400, 120 roots), 600-cell
  realisation, Snub 24-cell `Z_3` tripartition as a bridge candidate,
  Dirac spectrum on the 600-cell reported as `empirical_fit`.
- **Blind spots:** no derivation of the Standard Model from geometry
  alone; no end-to-end bridge `geometry -> spectral triple -> gauge ->
  Lagrangian`; `a4` conversion factor not fully reconciled across the
  three derivations.
- **Next unlock:** geometry atlas (one page per geometric fact, with its
  status), `a4` reconciliation quest, explicit boundary doc separating
  pure-math facts from physics-dependent ones.

### L4 - NCG / Lagrangian
- **Checked:** Connes-Chamseddine spectral-action benchmark as the
  comparison target; the derivation chain that the project would need
  to close is enumerated.
- **Blind spots:** gauge structure, Higgs sector, Yukawa couplings, and
  the three-generation structure are not derived; BT-2 (no sigma-field
  from H4) and BT-6 (no string / orbifold rescue) bound what can be
  recovered.
- **Next unlock:** finite-algebra candidate, Dirac-operator construction,
  inner-fluctuations check, side-by-side NCG comparison table, and a
  "no derivation, no claim" gate on the Lagrangian docs.

### L5 - Numerical Fits and Validation
- **Checked:** 26 core + 33 extended formulas matching PDG 2024 / NuFit 6.0 inside stated windows
  (interval-bound `Qed.` in `proofs/trinity/Catalog42.v`), falsifiability protocol, RG running scheme.
- **Honest p-value (Wave 20, 500k trials):** mean rel. error p = 0.077 (not significant);
  SG-hit density p < 0.0001 (highly significant). Random search matches average precision but rarely achieves the same density of ultra-precise hits.
- **σ-ranking:** median σ-distance = 0.085σ; 13/26 within 0.1σ; 3 "ultra-precision traps" (G01, Pr, L01) fail on σ only because measurements are 10⁵–10⁶× more precise than the formulas.
- **Blind spots:** `e^2` (electric charge) derivation, full RG closure,
  numerology / look-elsewhere risk, post-data prediction risk, G03 (sin²θ_W) is a genuine 84σ failure.
- **Next unlock:** fit registry tied to the claim ledger, systematic
  ablations, RG quest, pre-registered prediction lock for any new
  numerical claim.

### L6 - GOLDEN CHAIN Game
- **Checked:** Rust + wasm GOLDEN CHAIN canvas (live on Pages), five-ring
  Cargo workspace with inward-only dependency tests, every tile carries
  a `ClaimStatus`, falsification floor (any falsified tile collapses the
  bridge), framing the game as "data + geometry -> bridge candidate" and
  not as proof.
- **Blind spots:** the game is an educational interface, not a proof
  engine; level catalogue and claim-ledger SSOT are not yet wired into
  the game's tile data; "the game proves something" must stay an
  explicit non-claim.
- **Next unlock:** levels 1..5 of the GOLDEN CHAIN puzzle, each level
  unlocked only when its underlying claim row reaches the required
  status.

### L7 - Publication
- **Checked:** TRIOS PhD-style pipeline (paper v2, seminar talk v2,
  arXiv tarball, Zenodo deposit plan, citation metadata), all public
  documents written in English.
- **Blind spots:** drift risk between README, paper PDF, and game tile
  text; possible Cyrillic / non-English leakage in older artefacts;
  secrets in the tree.
- **Next unlock:** generated publication artefacts (README / paper / game
  catalogue all driven from `claims.yaml`), strict no-Russian gate on
  public docs, automated secret-scan gate.

> The diagram and the per-layer bullets above are intentionally a
> *summary*. Status per row, file pointers, and next milestones live in
> [`docs/TECH_TREE.md`](docs/TECH_TREE.md); when the two disagree, the
> TECH_TREE file is the source of truth and this section is updated.

---

## Track B — Cl(p,q) formalization (Wave 12 launch)

Following the boundary results on the H4/600-cell side, Wave 12 launches a
parallel **Track B** investigating Cl(8) / J3(O) / triality as an
alternative basis for the three-generations question (per
`outputs/B_program_T1_T12.md`).

The new directory [`proofs/clifford_cl8/`](proofs/clifford_cl8/) introduces:

- **T1** — definition of Cl(p,q) via universal property
  (`CliffordAlgebra.v`) — *mostly Qed*
- **T2** — statement of Cl(0,6) ≅ M_8(R) ⊕ M_8(R) (`Cl6_iso_M8R.v`) —
  *stated, Admitted with citation* (Lounesto 2001 Table 16.3, Wieser-Song
  2022 §6, arXiv:2110.03551)
- **T3** — statement of Bott 8-periodicity Cl(n+8) ≅ Cl(n) ⊗ Cl(8)
  (`Cl8_periodicity.v`) — *stated, Admitted with citation*
  (Atiyah-Bott-Shapiro 1964 Table 3, Lawson-Michelsohn 1989 Prop. I.4.1)

Track B is intentionally decoupled from the H4 core (no `Trinity` imports);
its Admitted set is logged separately as `TRACK_B_CLIFFORD` in
[admitted_log.md](docs/analysis/admitted_log.md). The launch PR provides the scaffolding
and statement-level theorems; T4–T12 and the Admitted discharges are
follow-up work.

---

## Trinity Fold — hypothesis-discovery puzzle prototype

[`games/trinity_fold/`](games/trinity_fold/) is a small, opt-in puzzle
prototype inspired by FoldIt, AlphaFold's Evoformer two-tower split, and
CASP-style held-out benchmarking. It scores candidate boards built from
theory tiles (constants, symmetries, geometry blocks, fields, constraints,
observables) against a fixed catalog and surfaces the **worst** claim
status across all active tiles. **It does not prove or test any Theory of
Everything claim.** Every tile carries a `ClaimStatus`
(`verified` / `empirical_fit` / `open_conjecture` /
`high_risk_or_falsified` / `unverified`), and boards containing
falsified tiles are floored at a negative total. See
[`games/trinity_fold/README.md`](games/trinity_fold/README.md) for the
honesty policy and scoring spec.

The prototype is fully decoupled from the Coq proofs and from the
`anti_numerology_check` CI gate; it lives in its own Rust **Cargo
workspace** (five ring crates: `ring0_core` ← `ring1_constraints` ←
`ring2_search` ← `ring3_adapters` ← `app`) so a broken game build never
blocks proof work and so each layer can be audited independently. The
inward-only dependency direction is enforced by integration tests in
`games/trinity_fold/crates/app/tests/ring_boundaries.rs`.

---

## The Four Boundary Theorems

| Theorem | Statement | File |
|---------|-----------|------|
| **BT-1** (Cosmology) | φ^a π^b e^c formulas cannot reproduce Λ or Ω_b | BoundaryTheorems.v |
| **BT-2** (σ-field) | No NCG σ-field from H4 root structure alone | BoundaryTheorems.v |
| **BT-3** (Chirality) | 600-cell D_F is vector-like (antipodal symmetry) | BoundaryTheorems.v |
| **BT-4** (Mass hierarchy) | 2I-equivariant D_F cannot reproduce lepton mass ratios | BoundaryTheorems.v |

---

## Coq Proof Statistics (Wave 15.1 — Honest Audit)

| Metric | Value |
|--------|-------|
| Coq version | 8.20.1 |
| Coq `.v` files (proofs + derivations) | **79** |
| `Qed.` theorems | **1 762** |
| Real `Admitted.` (outside comments/strings) | **5** |
| `Axiom` + `Conjecture` + `Parameter` | **85** |
| Refutation theorems | **14** |
| SG-class formulas (error < 0.01 %) | 11 |
| V-class formulas (error 0.01–0.3 %) | 12 |
| Honest p-value (500k trials) | p = 0.077 (mean error); p < 0.0001 (SG-hit density) |
| Anti-numerology gate | ✓ PASS (59 formulas tagged) |
| Honest-count script | [`scripts/count_admitted_honest.py`](scripts/count_admitted_honest.py) |

> **Audit note (Wave 15.1):** Prior counts mixed comment text with real proof
> obligations. A naive `grep -c "Admitted"` finds **77** matches in
> `proofs/trinity/`, but **all 77 are inside historical comments** (tags,
> TODOs, cross-references). The honest parser strips comments and strings
> before counting.  The result: `proofs/trinity/` has **0** real `Admitted.`,
> `proofs/clifford_cl8/` has **4**, and `derivations/chirality/` has **1**.
> Full methodology and commitment are in [HONESTY_MANIFEST.md](HONESTY_MANIFEST.md).
> All 5 open gaps are documented with citations or `[OPEN_PROBLEM]` labels.

---

## Build Instructions

### Prerequisites

- Coq 8.20.1
- opam (OCaml package manager)
- coq-interval, coq-coquelicot (Coq libraries)

### Local build

```bash
# Install dependencies (first time only)
opam repo add coq-released https://coq.inria.fr/opam/released
opam install coq.8.20.1 coq-interval coq-coquelicot

# Build
cd proofs/trinity
coq_makefile -f _CoqProject -o Makefile.coq
make -f Makefile.coq -j$(nproc)
```

### Docker

```bash
docker run -it --rm -v $(pwd):/work coqorg/coq:8.20.1-ocaml-4.14-flambda bash
cd /work/proofs/trinity
coq_makefile -f _CoqProject -o Makefile.coq
make -f Makefile.coq -j4
```

### Python validators

```bash
pip install mpmath numpy
python3 scripts/validators/validate_v4.py     # formula error bounds
python3 scripts/anti_numerology_gate.py       # honesty check
```

---

## Repository Structure

```
proofs/trinity/              — Coq .v source files (50 files)
  CorePhi.v                  — Golden ratio φ and its algebra
  Catalog42.v                — 42 SM parameter formulas [phenomenological_fit]
  BoundaryTheorems.v             — BT-1–BT-4 formal boundary theorems
  HiggsPrediction.v          — Higgs mass formulas [phenomenological_fit]
  CosmologyOrigins.v         — Cosmological formulas [HONEST: ...]
  ...
scripts/
  count_admitted_honest.py   — Honest parser: strips comments before counting
  anti_numerology_gate.py    — CI gate: detect untagged φ/π/e formulas
  README_anti_numerology.md  — Gate documentation
  prepare_zenodo.md          — Zenodo publication guide
.github/
  workflows/ci.yml           — CI: anti_numerology_check → build
  workflows/release.yml      — Release: coq bundle + PDF
  PULL_REQUEST_TEMPLATE.md   — Enforces "verified/open" PR section
RESEARCH_STATUS.md                   — Honest summary of results
CITATION.cff                 — Citation metadata (Zenodo)
CONTRIBUTING.md              — Contribution rules (Russian + English)
```

---

## Honesty Statement

The 59 formulas in `Catalog42.v` and related files match Standard Model parameters to <0.01% precision using combinations of φ, π, and e. **This is a catalog of numerical coincidences, not a physical theory.** The Boundary theorems prove that these coincidences cannot be elevated to a consistent NCG model of the Standard Model.

Every formula is tagged with `[phenomenological_fit]` and is automatically checked by the anti-numerology CI gate (`scripts/anti_numerology_gate.py`). Any new formula without an approved honesty tag will be rejected by CI.

---

## Citation

See [CITATION.cff](CITATION.cff) for citation metadata.

After Zenodo deposit, the DOI badge above will be updated. For now, cite the GitHub repository:

```bibtex
@software{trinity_s3ai_2026,
  title  = {Trinity-s3ai: An Active Boundary-Mapping Research Program on H4-Based Standard Model Unification},
  author = {[Author Name]},
  year   = {2026},
  url    = {https://github.com/gHashTag/trinity-s3ai},
  note   = {Version v1.0-wave10}
}
```

Also see [CITATION.bib](CITATION.bib) for formatted references.

---

## Wave History

| Wave | Key addition |
|------|-------------|
| 1–3 | Initial φ/π/e formula catalog |
| 4.1 | Honesty tags: `[phenomenological_fit]`, `[NUMERICAL_FIT]` added |
| 5–8 | NCG derivations, spectral action |
| 9.6 | BT-1–BT-4 boundary theorems formalized |
| 10.5 | Anti-numerology CI gate; CITATION.cff; CONTRIBUTING.md |
| 12 | Track B launch: Cl(p,q) formalization (T1–T3) |
| 15.1 | Honest counting system: comments stripped before statistics |
| 17 | Formal retraction of δ_CP = 65.66° (5.6σ excluded); 0 real `Admitted.` in proofs/trinity/ |
| 18 | Merge of honesty-pass PRs #21–#23 (Lagrangian status, δ_CP withdrawal, Coq audit) |
| 19 | Merge of honesty-pass PRs #29, #31, #32 (calculation-primacy, N_gen=3 withdrawn, Strong CP withdrawn); all old PRs merged |
| 20 | Honest phenomenology refresh: 500k-trial p-value, σ-ranking updated (26 obs, δ_CP withdrawn, sin²θ_13/23/W + |V_ub| + λ added) |
