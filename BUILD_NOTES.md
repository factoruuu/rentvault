# Build Notes — RentVault (Tari November Build Contest, theme: MVP)

Build date: 20 November 2026. Build only — nothing has been posted, submitted, or
published anywhere. No accounts or wallets were created.

## Deliverables

| Path | What |
|---|---|
| `template/` | Rust Ootle template crate (`rent_vault`): `src/lib.rs`, `tests/test.rs`, `Cargo.toml`, `template.toml` |
| `demo/index.html` | Single-file walletless browser simulation (no build step, no dependencies) |
| `README.md` | Project README (the document a judge reads first) |
| `LICENSE` | MIT |
| `BUILD_NOTES.md` | This file |

## Environment and installs

The build machine had no Rust toolchain. Installed:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
rustup target add wasm32-unknown-unknown
rustup component add clippy rustfmt   # rustfmt was already present; clippy added
```

Result: `cargo 1.100.0`, `rustc 1.100.0`, target `wasm32-unknown-unknown`.

Dependency versions (resolved by Cargo from the official scaffold's pins):
`tari_template_lib 0.40.1`, `tari_template_test_tooling 0.53.5`, `tari_engine 0.53.5`.

## Network problems and how they were solved

crates.io transfers from this machine stall intermittently ("transfer too slow: failed
to transfer more than 10 bytes in 30s"). This killed the first `cargo install
cargo-generate` and the first `cargo test` (dependency download phase).

- **cargo-generate:** after the install failed, the crate was scaffolded **manually**
  from the official scaffold source instead — `git clone
  https://github.com/tari-project/wasm-template`, then the `wasm_templates/empty`
  template copied with its placeholders substituted (`rent_vault` / `RentVault`). The
  result is byte-for-byte the structure `cargo generate` would have produced. The docs
  explicitly allow manual creation ("You can do this manually with `cargo new`").
- **Dependency downloads:** a retry loop around `cargo fetch` with
  `CARGO_HTTP_MULTIPLEXING=false` succeeded on the first retry pass (~43 MB cached);
  after that, all builds were fully local.

## Commands that work

```sh
cd template
cargo test                                   # full engine test suite
cargo build --target wasm32-unknown-unknown  # template WASM (debug)
cargo build --release --target wasm32-unknown-unknown
cargo fmt                                    # formatting applied
cargo clippy --all-targets                   # zero warnings in this crate
```

Release WASM size: **142,808 bytes (~139 KiB)**.

## Test results

Final run, after formatting and the review pass:

```
running 20 tests
test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 25.42s
```

Every state transition of the RentVault state machine has at least one passing test,
including all the negative cases. Test names:

**Creation & deposit**
1. `deposit_locks_the_agreed_amount_and_marks_the_agreement_funded`
2. `create_with_a_zero_deposit_is_rejected`
3. `deposit_with_the_wrong_amount_is_rejected`
4. `deposit_by_the_owner_is_rejected` (unauthorized caller — access rule denies)
5. `a_second_deposit_is_rejected`

**Mutual release**
6. `mutual_release_refunds_the_renter_in_full`
7. `mutual_release_works_when_the_owner_confirms_first`
8. `confirming_twice_is_rejected`
9. `a_stranger_cannot_confirm_the_return` (unauthorized caller)

**Owner claims**
10. `a_claim_before_the_rental_ends_is_rejected` (too-early claim)
11. `a_claim_by_the_renter_is_rejected` (unauthorized caller)
12. `a_claim_above_the_deposit_is_rejected`
13. `a_claim_of_zero_is_rejected`
14. `a_claim_pays_the_owner_and_refunds_the_remainder_after_the_dispute_window`
15. `a_full_deposit_claim_leaves_no_remainder`
16. `executing_a_claim_during_the_dispute_window_is_rejected`

**Disputes**
17. `a_dispute_during_the_window_voids_the_claim_and_refunds_the_renter`
18. `a_dispute_after_the_window_is_rejected_and_the_claim_still_pays`

**Auto-refund**
19. `an_unclaimed_deposit_is_auto_refunded_after_the_claim_window`
20. `an_auto_refund_before_the_claim_window_closes_is_rejected`

Notes on test design: the harness disables transaction fees by default, so account
balances are asserted **exactly** after every flow. Settlement triggers are executed by
a third "stranger" account in tests 14 and 19, which also assert the stranger's balance
is unchanged — proving the permissionless triggers cannot be abused for gain.

## Demo verification

`demo/index.html` was verified beyond eyeballing:

- The embedded JavaScript passes `node --check`.
- A headless Node run with a minimal DOM stub drove the full state machine: all four
  settlement paths (mutual release, claim paid 80/170 split, dispute refund,
  auto-refund) end in the exact balances the template tests assert, and the rejection
  paths (owner attempting deposit, claim before rental end) refuse correctly.

## Deviations from the docs (and why)

1. **Manual scaffold instead of `cargo generate`** — see "Network problems" above.
2. **Struct declared before the enums in `src/lib.rs`.** The `#[template]` macro takes
   the *first* struct/enum in the module as the template name; with `Status` declared
   first, the WASM build fails with "a template must have associated functions and/or
   methods". The struct now comes first, with a comment in the code explaining why.
   (The host-target build compiles either way — only the WASM macro pass catches it,
   which the engine tests exercise.)
3. **`tari_template_lib` 0.40, not 0.41.** The docs text mentions 0.41 as the latest
   documented version, but the official scaffold pins 0.40 and pairs it with
   `tari_template_test_tooling` 0.53; the scaffold's tested pairing was kept rather
   than mixing versions.
4. **Dispute rule choice.** Of the options in the design brief, the implemented rule is
   "dispute voids the claim and refunds the renter in full" — the simplest defensible
   rule, documented in the README ("The dispute rule, honestly") and in the template's
   doc comments.

## Self-review pass (judge's-eye)

After the first green run, the code was re-read as a reviewer would:

- `cargo fmt` applied; `cargo clippy --all-targets` reports **zero warnings** in this
  crate. (One `#[allow(clippy::too_many_arguments)]` on the constructor, with a comment:
  a template constructor's parameters *are* the transaction arguments and cannot be
  grouped into a config struct.)
- No `unwrap()`/`expect()` anywhere in template code; all rejections are `assert!` /
  `panic!` with user-readable messages, which the tests assert verbatim.
- No dead code; every public method is part of the agreement API and is exercised or
  read by the tests/demo.
- Two assertions were *added* during review (stranger's balance unchanged after
  triggering settlement) rather than silencing a dead-code warning on the test setup.
- Amounts use checked arithmetic from the template library; the only subtraction
  (claim remainder) is guarded by `claim <= deposit` at filing time and by the vault's
  actual balance at execution time.

## Still missing for submission (deliberately not done)

Per the contest's submission checklist, the following are still needed — none were
created or posted, per instructions:

1. **A public code repository** (e.g. GitHub) containing this tree.
2. **A Tari payment address** to receive winnings (requires creating a Tari wallet —
   not done).
3. **A forum account and one submission post** in the November contest thread
   (community.tari.com thread 520) containing: code link, MVP app link, description,
   payment address, announcement link.
4. **An announcement post** on Bluesky, Reddit (r/tari), Medium, or YouTube — X is not
   accepted. (The CEO has a Medium account, which is the lowest-friction option.)
5. Optional but strengthening: publish the template to the Esmeralda testnet and link
   the explorer record; host `demo/` on GitHub Pages so the "MVP app" link is live
   rather than a file in the repo.

## Why this can win

Measured against the 28 November entries already posted:

- **A real-world problem everyone has had.** Most entries are crypto-native plumbing
  (escrows for bounties, splitters, tip jars, membership passes). RentVault is about
  rental deposits — a judge needs zero domain background to feel the problem, and the
  "MVP you could go to market with today" test is met literally: the rental flow it
  serves already exists, in cash, everywhere.
- **Completeness of the state machine.** Several entries implement a single happy path
  (one entry ships a Python reference engine with the template listed as roadmap).
  RentVault implements and tests *every* exit — including the two failure modes that
  actually lose people money today (owner keeps the deposit by default; owner goes
  silent) — with 20 engine tests, more than double the test count of the strongest
  comparable entries (8), all asserting exact balances.
- **A security story that is designed, not decorated.** `OwnerRule::None` (the
  component has no owner at all), double gating (access rules + in-method signer
  checks), locked rules, exact-amount native-token deposits, and permissionless
  settlement triggers that provably pay the caller nothing. Each choice is explained in
  the README's security section and enforced by a test.
- **Honesty as presentation.** The README names its own trade-off (the dispute rule)
  instead of hiding it, separates "real" from "simulated" precisely, and states plainly
  what is *not* done (no testnet publish yet). The strongest entries in the thread do
  exactly this; weaker ones oversell.
- **Judge-proof demo.** The demo needs no wallet, no instructions, and no imagination:
  four labelled scenarios, role switching, an epoch clock, and an event log that shows
  rejections with the same messages the template produces — verified headlessly against
  the same flows the engine tests assert.
