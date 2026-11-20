# RentVault — rental security deposits, held by code, not by trust

**RentVault is an escrow for rental security deposits on Tari Ootle.** A renter locks a
deposit into a RentVault component before taking an item home. From that moment, neither
side can touch the money: it can only leave the vault through rules both parties agreed
to up front — a mutual release, an owner's claim that survives a dispute window, or an
automatic refund if the owner never claims.

No platform holds the funds. No one's goodwill is required to get a deposit back. And a
deposit can never silently disappear into an owner's pocket or get stuck forever.

## The problem

Security deposits run on trust today, and the trust is one-sided. Rent a camera, a car, a
power tool, a karaoke machine for the weekend: you hand over cash (or a card hold) to a
stranger, and getting it back depends on that stranger's honesty and memory. Owners, in
turn, have no protection when an item genuinely comes back damaged — the argument happens
over cash, in person, with no record.

The usual fixes don't fix it. A marketplace can hold deposits, but then the marketplace
*is* the trusted party, takes a cut, and only covers its own listings. A plain multisig
or a "send it to this address" arrangement has no deadlines: if the owner simply never
responds, the renter's money is stranded.

## How it works

One agreement = one `RentVault` component, created by the renter for a specific rental:

| | |
|---|---|
| **Parties** | A **renter** (creates the agreement, locks the deposit) and an **owner** (the counterparty renting out the item) |
| **Terms, fixed at creation** | Deposit amount · rental end epoch · dispute window · claim window |
| **Clock** | Epochs, read on-chain via `Consensus::current_epoch()` |

```
                 deposit            both confirm return
  AwaitingDeposit ──────► Funded ───────────────────────► Settled (MutualRelease)
                            │
                            │ owner claims (after rental end)
                            ▼
                       ClaimPending ── window passes ──► Settled (ClaimPaid:
                            │                              owner ← claim, renter ← rest)
                            │ renter disputes in window
                            └──────────────────────────► Settled (DisputeRefund)
                            │
  Funded ── claim window passes with no claim ─────────► Settled (AutoRefund)
```

The four ways out — and there are exactly four, so funds always have an exit:

1. **Mutual release.** Both parties confirm the item came back in good condition (in
   either order). The deposit refunds to the renter in full.
2. **Owner claim.** After the rental ends, the owner may claim up to the full deposit,
   e.g. for damage. The claim opens a **dispute window** instead of paying out.
3. **Dispute.** During the window the renter can void the claim, refunding the deposit
   in full. If the window passes undisputed, **anyone** can execute the claim: the owner
   receives the claimed amount, the renter the remainder.
4. **Auto-refund.** If the owner never claims within the claim window, **anyone** can
   trigger a full refund to the renter. The renter never has to chase the owner — the
   deadline does the work.

**The dispute rule, honestly.** RentVault is a custody layer, not a court. If a claim is
contested, the on-chain default is to return the deposit and move the argument off-chain,
where the claim and the dispute are both on the public record as evidence — the same
shape as a card chargeback. What the chain guarantees is the thing renters actually lose
today: an owner can never keep a deposit *silently, by default, or by going missing*.

## Why this is an MVP you could launch today

- **The market exists now.** Peer-to-peer and small-shop rentals — cameras, tools,
  vehicles, event gear, instruments — move real deposits every day, in cash, on trust.
  RentVault drops into that flow unchanged: agree terms, lock, hand over the item.
- **The product is the template.** A rental shop or marketplace front-end needs exactly
  four calls to integrate (create, deposit, confirm/claim, settle). Everything else —
  listings, identity, messaging — already exists in the apps people use.
- **No operator.** There is no admin key, no fee switch, no pause button. Once created,
  an agreement settles by its own rules; a business built on RentVault never custodies
  customer funds and never has to be trusted with them.
- **Deadlines are the feature.** The dispute and claim windows convert the two ways
  deposits die today — "the owner kept it because they could" and "the owner vanished" —
  into outcomes the renter can force alone.

## Security design

- **No component owner.** The component is created with `OwnerRule::None` and locked
  access rules. Not even the renter who created the agreement has implicit privileges;
  the creator of an agreement cannot rewrite its rules, ever.
- **Two gates on every restricted method.** Each state-changing method is protected by a
  component access rule naming the authorised party's public key (`rule!(public_key(..))`)
  *and* by an explicit signer check inside the method. Either layer alone is sufficient
  to stop a wrong-party call; both are tested.
- **Exact-amount, native-token deposits.** `deposit` rejects anything that is not
  exactly the agreed amount of the native TARI token, so funds can't be locked in the
  wrong amount or asset.
- **No stranded states.** Every non-settled state has a permissionless or counterparty
  exit, and settlement triggers (`execute_claim`, `trigger_refund`) are callable by
  anyone — they can only move funds to their pre-agreed destinations, and tests assert
  the caller gains nothing by calling them.
- **Events as the paper trail.** Agreements emit `rent_vault.agreement_created`,
  `rent_vault.deposit_locked`, `rent_vault.return_confirmed`, `rent_vault.claim_filed`,
  `rent_vault.claim_disputed` and `rent_vault.settled`, so indexers and front-ends can
  follow an agreement without polling state.

## What's included

- **`template/`** — the RentVault template (Rust, `tari_template_lib` 0.40), one
  component, no external dependencies beyond the template library. Release WASM:
  **139 KiB**. Formatted with `rustfmt`, clean under `cargo clippy`.
- **20 engine tests** (`template/tests/test.rs`) using `tari_template_test_tooling` —
  the real Ootle engine, not a mock. They cover every state transition above and the
  failure modes: wrong party, wrong amount, second deposit, double confirmation,
  claim before rental end, claim above the deposit, zero claim, execution during the
  dispute window, late dispute, early auto-refund. Balances are asserted exactly
  (the harness charges no fees by default).
- **`demo/`** — a walletless browser simulation of the full flow (below).

## Run it

**Tests** (needs Rust + the `wasm32-unknown-unknown` target):

```sh
cd template
cargo test
```

Expected: `test result: ok. 20 passed; 0 failed`.

**Demo:** live at https://longan.celurut.online/rentvault/ — or open `demo/index.html` in any browser — no build step, no wallet, no network.
Pick a scenario at the top, switch between *Renter / Owner / Anyone* and click through
the flow. The demo runs the same state machine as the template, with the same rejection
messages, and logs the events the chain would emit.

## What is real, and what is simulated

- **Real:** the template and its engine tests. The template compiles to WASM and every
  rule described above is enforced by code the tests exercise end-to-end.
- **Simulated:** the browser demo. It mirrors the template's state machine in JavaScript
  so judges can click through the product without a wallet; it is not connected to a
  network.
- **Not yet done:** the template has not been published to Esmeralda or mainnet, and
  there is no wallet-connected front-end. The natural next steps are publishing to
  Esmeralda, a thin web front-end over the wallet daemon, and — once a marketplace
  partner exists — per-listing agreement creation from the listing page. A production
  version would also evaluate Ootle's confidential amounts so deposit sizes aren't
  public, and item receipts as NFTs bound to the agreement.

## License

MIT — see `LICENSE`.

*Built with an AI agent, as the contest rules explicitly allow. The idea, direction and
submission are the entrant's own; the code is new, written for this contest.*
