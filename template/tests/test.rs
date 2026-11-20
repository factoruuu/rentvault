//! Engine tests for the RentVault template.
//!
//! Every test runs real transactions against the template compiled to WASM, using the
//! same Ootle engine as the network (`tari_template_test_tooling`). Fees are disabled
//! in the harness by default, so account balances can be asserted exactly.
//!
//! Coverage: every state transition of the agreement lifecycle, plus the negative
//! cases — wrong party, wrong amount, too-early claim, late dispute, early refund.

use tari_template_lib::types::Amount;
use tari_template_test_tooling::TemplateTest;
use tari_template_test_tooling::byte_type::ToByteType;
use tari_template_test_tooling::crypto::{PublicKey, RistrettoPublicKey, RistrettoSecretKey};
use tari_template_test_tooling::engine_types::virtual_substate::{
    VirtualSubstate, VirtualSubstateId,
};
use tari_template_test_tooling::support::assert_error::assert_reject_reason;
use tari_template_test_tooling::template_lib_types::{ComponentAddress, constants::TARI_TOKEN};
use tari_template_test_tooling::transaction::args;

/// The agreed deposit used across the tests: 250 TARI (amounts are in µTari).
const DEPOSIT: u64 = 250_000_000;
/// The owner's claim used across the tests: 80 TARI.
const CLAIM: u64 = 80_000_000;
/// The rental ends at this epoch; claims open from here.
const RENTAL_END: u64 = 5;
/// A claim filed at epoch E is disputable up to and including epoch E + 3.
const DISPUTE_WINDOW: u64 = 3;
/// The owner may claim up to and including epoch RENTAL_END + 4.
const CLAIM_WINDOW: u64 = 4;
/// What every funded test account starts with.
const START_BALANCE: u64 = TemplateTest::FUNDED_ACCOUNT_INITIAL_BALANCE;

struct Setup {
    test: TemplateTest,
    component: ComponentAddress,
    renter: ComponentAddress,
    renter_key: RistrettoSecretKey,
    owner: ComponentAddress,
    owner_key: RistrettoSecretKey,
    stranger: ComponentAddress,
    stranger_key: RistrettoSecretKey,
}

/// Creates the three parties and a fresh (unfunded) agreement between renter and owner.
fn setup() -> Setup {
    let mut test = TemplateTest::my_crate();
    let template = test.get_template_address("RentVault");

    let (renter, _renter_proof, renter_key) = test.create_funded_account();
    let (owner, _owner_proof, owner_key) = test.create_funded_account();
    let (stranger, _stranger_proof, stranger_key) = test.create_funded_account();

    let owner_public_key = RistrettoPublicKey::from_secret_key(&owner_key).to_byte_type();

    test.execute_expect_success(
        test.transaction()
            .call_function(
                template,
                "new",
                args![
                    "Cinema camera kit".to_string(),
                    renter,
                    owner,
                    owner_public_key,
                    Amount::from(DEPOSIT),
                    RENTAL_END,
                    DISPUTE_WINDOW,
                    CLAIM_WINDOW
                ],
            )
            .build_and_seal(&renter_key),
        vec![],
    );

    let (component, _) = test
        .read_only_state_store()
        .get_components_by_template_address(template)
        .unwrap()
        .remove(0);

    Setup {
        test,
        component,
        renter,
        renter_key,
        owner,
        owner_key,
        stranger,
        stranger_key,
    }
}

/// A setup whose agreement already holds the deposit.
fn setup_funded() -> Setup {
    let mut s = setup();
    deposit(&mut s);
    s
}

/// The renter locks the agreed deposit into the component's vault.
fn deposit(s: &mut Setup) {
    let tx = s
        .test
        .transaction()
        .call_method(
            s.renter,
            "withdraw",
            args![TARI_TOKEN, Amount::from(DEPOSIT)],
        )
        .put_last_instruction_output_on_workspace("payment")
        .call_method(s.component, "deposit", args![Workspace("payment")])
        .build_and_seal(&s.renter_key);
    s.test.execute_expect_success(tx, vec![]);
}

/// Moves the chain clock the template reads (`Consensus::current_epoch`).
fn set_epoch(test: &mut TemplateTest, epoch: u64) {
    test.set_virtual_substate(
        VirtualSubstateId::CurrentEpoch,
        VirtualSubstate::CurrentEpoch(epoch),
    );
}

/// Reads a `String`-returning getter from the component.
fn get_string(s: &mut Setup, method: &str) -> String {
    let tx = s
        .test
        .transaction()
        .call_method(s.component, method, args![])
        .build_and_seal(s.test.secret_key());
    s.test.execute_expect_success(tx, vec![]);
    s.test.expect_return::<String>(0)
}

/// Reads an `Amount`-returning getter from the component.
fn get_amount(s: &mut Setup, method: &str) -> Amount {
    let tx = s
        .test
        .transaction()
        .call_method(s.component, method, args![])
        .build_and_seal(s.test.secret_key());
    s.test.execute_expect_success(tx, vec![]);
    s.test.expect_return::<Amount>(0)
}

/// The TARI balance of an account.
fn balance(test: &TemplateTest, account: ComponentAddress) -> Amount {
    let vaults = test
        .read_only_state_store()
        .get_vaults_for_account(account)
        .unwrap();
    vaults
        .get(&TARI_TOKEN)
        .expect("account should hold a TARI vault")
        .balance()
}

// ---------------------------------------------------------------------------
// Creation and deposit
// ---------------------------------------------------------------------------

#[test]
fn deposit_locks_the_agreed_amount_and_marks_the_agreement_funded() {
    let mut s = setup();
    assert_eq!(get_string(&mut s, "status"), "AwaitingDeposit");
    assert_eq!(get_string(&mut s, "outcome"), "None");

    deposit(&mut s);

    assert_eq!(get_string(&mut s, "status"), "Funded");
    assert_eq!(get_amount(&mut s, "vault_balance"), Amount::from(DEPOSIT));
    assert_eq!(
        balance(&s.test, s.renter),
        Amount::from(START_BALANCE - DEPOSIT)
    );
    assert_eq!(balance(&s.test, s.owner), Amount::from(START_BALANCE));
}

#[test]
fn create_with_a_zero_deposit_is_rejected() {
    let mut test = TemplateTest::my_crate();
    let template = test.get_template_address("RentVault");
    let (renter, _proof, renter_key) = test.create_funded_account();
    let (owner, _proof, owner_key) = test.create_funded_account();
    let owner_public_key = RistrettoPublicKey::from_secret_key(&owner_key).to_byte_type();

    let reason = test.execute_expect_failure(
        test.transaction()
            .call_function(
                template,
                "new",
                args![
                    "Cinema camera kit".to_string(),
                    renter,
                    owner,
                    owner_public_key,
                    Amount::zero(),
                    RENTAL_END,
                    DISPUTE_WINDOW,
                    CLAIM_WINDOW
                ],
            )
            .build_and_seal(&renter_key),
        vec![],
    );

    assert_reject_reason(reason, "Deposit amount must be positive");
}

#[test]
fn deposit_with_the_wrong_amount_is_rejected() {
    let mut s = setup();

    let tx = s
        .test
        .transaction()
        .call_method(
            s.renter,
            "withdraw",
            args![TARI_TOKEN, Amount::from(DEPOSIT - 1)],
        )
        .put_last_instruction_output_on_workspace("payment")
        .call_method(s.component, "deposit", args![Workspace("payment")])
        .build_and_seal(&s.renter_key);
    let reason = s.test.execute_expect_failure(tx, vec![]);

    assert_reject_reason(reason, "Deposit must be exactly the agreed amount");
    // The rejected transaction committed nothing: no funds moved, still awaiting deposit.
    assert_eq!(get_string(&mut s, "status"), "AwaitingDeposit");
    assert_eq!(balance(&s.test, s.renter), Amount::from(START_BALANCE));
}

#[test]
fn deposit_by_the_owner_is_rejected() {
    let mut s = setup();

    // The owner tries to fund the agreement themselves. The component's access rules
    // name the renter's key for `deposit`, so this is denied before any logic runs.
    let tx = s
        .test
        .transaction()
        .call_method(
            s.owner,
            "withdraw",
            args![TARI_TOKEN, Amount::from(DEPOSIT)],
        )
        .put_last_instruction_output_on_workspace("payment")
        .call_method(s.component, "deposit", args![Workspace("payment")])
        .build_and_seal(&s.owner_key);
    s.test.execute_expect_failure(tx, vec![]);

    assert_eq!(get_string(&mut s, "status"), "AwaitingDeposit");
    assert_eq!(balance(&s.test, s.owner), Amount::from(START_BALANCE));
}

#[test]
fn a_second_deposit_is_rejected() {
    let mut s = setup_funded();

    let tx = s
        .test
        .transaction()
        .call_method(
            s.renter,
            "withdraw",
            args![TARI_TOKEN, Amount::from(DEPOSIT)],
        )
        .put_last_instruction_output_on_workspace("payment")
        .call_method(s.component, "deposit", args![Workspace("payment")])
        .build_and_seal(&s.renter_key);
    let reason = s.test.execute_expect_failure(tx, vec![]);

    assert_reject_reason(reason, "The deposit has already been made");
    assert_eq!(get_amount(&mut s, "vault_balance"), Amount::from(DEPOSIT));
}

// ---------------------------------------------------------------------------
// Mutual release
// ---------------------------------------------------------------------------

#[test]
fn mutual_release_refunds_the_renter_in_full() {
    let mut s = setup_funded();

    // Renter confirms first: nothing settles until both parties have confirmed.
    let tx = s
        .test
        .transaction()
        .call_method(s.component, "confirm_return", args![])
        .build_and_seal(&s.renter_key);
    s.test.execute_expect_success(tx, vec![]);
    assert_eq!(get_string(&mut s, "status"), "Funded");

    // Owner confirms: the deposit goes straight back to the renter.
    let tx = s
        .test
        .transaction()
        .call_method(s.component, "confirm_return", args![])
        .build_and_seal(&s.owner_key);
    s.test.execute_expect_success(tx, vec![]);

    assert_eq!(get_string(&mut s, "status"), "Settled");
    assert_eq!(get_string(&mut s, "outcome"), "MutualRelease");
    assert_eq!(get_amount(&mut s, "vault_balance"), Amount::zero());
    assert_eq!(balance(&s.test, s.renter), Amount::from(START_BALANCE));
    assert_eq!(balance(&s.test, s.owner), Amount::from(START_BALANCE));
}

#[test]
fn mutual_release_works_when_the_owner_confirms_first() {
    let mut s = setup_funded();

    let tx = s
        .test
        .transaction()
        .call_method(s.component, "confirm_return", args![])
        .build_and_seal(&s.owner_key);
    s.test.execute_expect_success(tx, vec![]);
    assert_eq!(get_string(&mut s, "status"), "Funded");

    let tx = s
        .test
        .transaction()
        .call_method(s.component, "confirm_return", args![])
        .build_and_seal(&s.renter_key);
    s.test.execute_expect_success(tx, vec![]);

    assert_eq!(get_string(&mut s, "status"), "Settled");
    assert_eq!(get_string(&mut s, "outcome"), "MutualRelease");
    assert_eq!(balance(&s.test, s.renter), Amount::from(START_BALANCE));
}

#[test]
fn confirming_twice_is_rejected() {
    let mut s = setup_funded();

    let tx = s
        .test
        .transaction()
        .call_method(s.component, "confirm_return", args![])
        .build_and_seal(&s.renter_key);
    s.test.execute_expect_success(tx, vec![]);

    let tx = s
        .test
        .transaction()
        .call_method(s.component, "confirm_return", args![])
        .build_and_seal(&s.renter_key);
    let reason = s.test.execute_expect_failure(tx, vec![]);

    assert_reject_reason(reason, "The renter has already confirmed the return");
}

#[test]
fn a_stranger_cannot_confirm_the_return() {
    let mut s = setup_funded();

    let tx = s
        .test
        .transaction()
        .call_method(s.component, "confirm_return", args![])
        .build_and_seal(&s.stranger_key);
    s.test.execute_expect_failure(tx, vec![]);

    // The stranger's attempt recorded nothing: the real parties can still settle.
    assert_eq!(get_string(&mut s, "status"), "Funded");
    let tx = s
        .test
        .transaction()
        .call_method(s.component, "confirm_return", args![])
        .build_and_seal(&s.renter_key);
    s.test.execute_expect_success(tx, vec![]);
    let tx = s
        .test
        .transaction()
        .call_method(s.component, "confirm_return", args![])
        .build_and_seal(&s.owner_key);
    s.test.execute_expect_success(tx, vec![]);
    assert_eq!(get_string(&mut s, "status"), "Settled");
}

// ---------------------------------------------------------------------------
// Owner claims
// ---------------------------------------------------------------------------

/// A funded setup with a pending claim of `CLAIM`, filed by the owner at RENTAL_END.
fn setup_with_claim() -> Setup {
    let mut s = setup_funded();
    set_epoch(&mut s.test, RENTAL_END);
    file_claim(&mut s, CLAIM);
    s
}

fn file_claim(s: &mut Setup, amount: u64) {
    let tx = s
        .test
        .transaction()
        .call_method(s.component, "file_claim", args![Amount::from(amount)])
        .build_and_seal(&s.owner_key);
    s.test.execute_expect_success(tx, vec![]);
}

#[test]
fn a_claim_before_the_rental_ends_is_rejected() {
    let mut s = setup_funded();
    // Still epoch 0; the rental ends at RENTAL_END.

    let tx = s
        .test
        .transaction()
        .call_method(s.component, "file_claim", args![Amount::from(CLAIM)])
        .build_and_seal(&s.owner_key);
    let reason = s.test.execute_expect_failure(tx, vec![]);

    assert_reject_reason(reason, "The rental has not ended yet");
    assert_eq!(get_string(&mut s, "status"), "Funded");
}

#[test]
fn a_claim_by_the_renter_is_rejected() {
    let mut s = setup_funded();
    set_epoch(&mut s.test, RENTAL_END);

    // Only the owner's key satisfies the `file_claim` access rule.
    let tx = s
        .test
        .transaction()
        .call_method(s.component, "file_claim", args![Amount::from(CLAIM)])
        .build_and_seal(&s.renter_key);
    s.test.execute_expect_failure(tx, vec![]);

    // The agreement is untouched: the owner can still file their claim.
    assert_eq!(get_string(&mut s, "status"), "Funded");
    file_claim(&mut s, CLAIM);
    assert_eq!(get_string(&mut s, "status"), "ClaimPending");
}

#[test]
fn a_claim_above_the_deposit_is_rejected() {
    let mut s = setup_funded();
    set_epoch(&mut s.test, RENTAL_END);

    let tx = s
        .test
        .transaction()
        .call_method(s.component, "file_claim", args![Amount::from(DEPOSIT + 1)])
        .build_and_seal(&s.owner_key);
    let reason = s.test.execute_expect_failure(tx, vec![]);

    assert_reject_reason(reason, "A claim cannot exceed the deposit");
}

#[test]
fn a_claim_of_zero_is_rejected() {
    let mut s = setup_funded();
    set_epoch(&mut s.test, RENTAL_END);

    let tx = s
        .test
        .transaction()
        .call_method(s.component, "file_claim", args![Amount::zero()])
        .build_and_seal(&s.owner_key);
    let reason = s.test.execute_expect_failure(tx, vec![]);

    assert_reject_reason(reason, "A claim must be for a positive amount");
}

#[test]
fn a_claim_pays_the_owner_and_refunds_the_remainder_after_the_dispute_window() {
    let mut s = setup_with_claim();
    assert_eq!(get_string(&mut s, "status"), "ClaimPending");
    assert_eq!(get_amount(&mut s, "claimed_amount"), Amount::from(CLAIM));

    // One epoch past the dispute deadline (claim at 5, window 3, deadline 8).
    set_epoch(&mut s.test, RENTAL_END + DISPUTE_WINDOW + 1);

    // Anyone may settle an undisputed claim — here, a complete stranger does it.
    let tx = s
        .test
        .transaction()
        .call_method(s.component, "execute_claim", args![])
        .build_and_seal(&s.stranger_key);
    s.test.execute_expect_success(tx, vec![]);

    assert_eq!(get_string(&mut s, "status"), "Settled");
    assert_eq!(get_string(&mut s, "outcome"), "ClaimPaid");
    assert_eq!(get_amount(&mut s, "vault_balance"), Amount::zero());
    // The stranger who triggered settlement gained nothing by doing so.
    assert_eq!(balance(&s.test, s.stranger), Amount::from(START_BALANCE));
    assert_eq!(
        balance(&s.test, s.owner),
        Amount::from(START_BALANCE + CLAIM)
    );
    assert_eq!(
        balance(&s.test, s.renter),
        Amount::from(START_BALANCE - CLAIM)
    );
}

#[test]
fn a_full_deposit_claim_leaves_no_remainder() {
    let mut s = setup_funded();
    set_epoch(&mut s.test, RENTAL_END);
    file_claim(&mut s, DEPOSIT);
    set_epoch(&mut s.test, RENTAL_END + DISPUTE_WINDOW + 1);

    let tx = s
        .test
        .transaction()
        .call_method(s.component, "execute_claim", args![])
        .build_and_seal(&s.owner_key);
    s.test.execute_expect_success(tx, vec![]);

    assert_eq!(get_string(&mut s, "outcome"), "ClaimPaid");
    assert_eq!(
        balance(&s.test, s.owner),
        Amount::from(START_BALANCE + DEPOSIT)
    );
    assert_eq!(
        balance(&s.test, s.renter),
        Amount::from(START_BALANCE - DEPOSIT)
    );
}

#[test]
fn executing_a_claim_during_the_dispute_window_is_rejected() {
    let mut s = setup_with_claim();

    // At the dispute deadline itself the renter may still dispute, so execution
    // must wait one more epoch.
    set_epoch(&mut s.test, RENTAL_END + DISPUTE_WINDOW);
    let tx = s
        .test
        .transaction()
        .call_method(s.component, "execute_claim", args![])
        .build_and_seal(&s.stranger_key);
    let reason = s.test.execute_expect_failure(tx, vec![]);

    assert_reject_reason(reason, "The dispute window is still open");
    assert_eq!(get_string(&mut s, "status"), "ClaimPending");
}

// ---------------------------------------------------------------------------
// Disputes
// ---------------------------------------------------------------------------

#[test]
fn a_dispute_during_the_window_voids_the_claim_and_refunds_the_renter() {
    let mut s = setup_with_claim();

    // Two epochs into the three-epoch dispute window.
    set_epoch(&mut s.test, RENTAL_END + 2);
    let tx = s
        .test
        .transaction()
        .call_method(s.component, "dispute_claim", args![])
        .build_and_seal(&s.renter_key);
    s.test.execute_expect_success(tx, vec![]);

    assert_eq!(get_string(&mut s, "status"), "Settled");
    assert_eq!(get_string(&mut s, "outcome"), "DisputeRefund");
    assert_eq!(get_amount(&mut s, "vault_balance"), Amount::zero());
    assert_eq!(balance(&s.test, s.renter), Amount::from(START_BALANCE));
    assert_eq!(balance(&s.test, s.owner), Amount::from(START_BALANCE));

    // The voided claim can no longer be executed.
    set_epoch(&mut s.test, RENTAL_END + DISPUTE_WINDOW + 1);
    let tx = s
        .test
        .transaction()
        .call_method(s.component, "execute_claim", args![])
        .build_and_seal(&s.stranger_key);
    let reason = s.test.execute_expect_failure(tx, vec![]);
    assert_reject_reason(reason, "There is no pending claim to execute");
}

#[test]
fn a_dispute_after_the_window_is_rejected_and_the_claim_still_pays() {
    let mut s = setup_with_claim();
    set_epoch(&mut s.test, RENTAL_END + DISPUTE_WINDOW + 1);

    let tx = s
        .test
        .transaction()
        .call_method(s.component, "dispute_claim", args![])
        .build_and_seal(&s.renter_key);
    let reason = s.test.execute_expect_failure(tx, vec![]);
    assert_reject_reason(reason, "The dispute window has closed");

    // The claim is unaffected and settles normally.
    let tx = s
        .test
        .transaction()
        .call_method(s.component, "execute_claim", args![])
        .build_and_seal(&s.stranger_key);
    s.test.execute_expect_success(tx, vec![]);
    assert_eq!(get_string(&mut s, "outcome"), "ClaimPaid");
    assert_eq!(
        balance(&s.test, s.owner),
        Amount::from(START_BALANCE + CLAIM)
    );
}

// ---------------------------------------------------------------------------
// Auto-refund
// ---------------------------------------------------------------------------

#[test]
fn an_unclaimed_deposit_is_auto_refunded_after_the_claim_window() {
    let mut s = setup_funded();

    // One epoch past the last moment the owner could have claimed (5 + 4).
    set_epoch(&mut s.test, RENTAL_END + CLAIM_WINDOW + 1);

    // Anyone may trigger the refund — here, a complete stranger does it.
    let tx = s
        .test
        .transaction()
        .call_method(s.component, "trigger_refund", args![])
        .build_and_seal(&s.stranger_key);
    s.test.execute_expect_success(tx, vec![]);

    assert_eq!(get_string(&mut s, "status"), "Settled");
    assert_eq!(get_string(&mut s, "outcome"), "AutoRefund");
    assert_eq!(get_amount(&mut s, "vault_balance"), Amount::zero());
    // The stranger who triggered the refund gained nothing by doing so.
    assert_eq!(balance(&s.test, s.stranger), Amount::from(START_BALANCE));
    assert_eq!(balance(&s.test, s.renter), Amount::from(START_BALANCE));
    assert_eq!(balance(&s.test, s.owner), Amount::from(START_BALANCE));
}

#[test]
fn an_auto_refund_before_the_claim_window_closes_is_rejected() {
    let mut s = setup_funded();

    // At the last epoch of the claim window the owner may still claim.
    set_epoch(&mut s.test, RENTAL_END + CLAIM_WINDOW);
    let tx = s
        .test
        .transaction()
        .call_method(s.component, "trigger_refund", args![])
        .build_and_seal(&s.stranger_key);
    let reason = s.test.execute_expect_failure(tx, vec![]);

    assert_reject_reason(reason, "The owner can still claim");
    assert_eq!(get_string(&mut s, "status"), "Funded");
    assert_eq!(get_amount(&mut s, "vault_balance"), Amount::from(DEPOSIT));
}
