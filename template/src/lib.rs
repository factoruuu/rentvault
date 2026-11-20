//! RentVault — an escrow for rental security deposits on Tari Ootle.
//!
//! A renter locks a deposit for a rented item into a RentVault component. The funds can
//! only ever leave the vault through the rules both parties agreed to when the agreement
//! was created:
//!
//! * **Mutual release** — renter and owner both confirm the item came back in good
//!   condition, and the deposit is refunded to the renter in full.
//! * **Owner claim** — after the rental ends, the owner may claim up to the full deposit
//!   (e.g. for damage). A claim only pays out once a dispute window has passed with no
//!   dispute from the renter; the unclaimed remainder is refunded to the renter.
//! * **Renter dispute** — during the dispute window the renter can void a claim, which
//!   refunds the deposit in full. RentVault is a custody layer, not a court: a contested
//!   deduction moves off-chain, with the claim and the dispute on the public record.
//! * **Auto-refund** — if the owner never claims within the claim window after the
//!   rental ends, anyone can trigger a full refund to the renter. Funds can never be
//!   stranded in the vault.
//!
//! The component deliberately has **no owner** (`OwnerRule::None`): not even the renter
//! who created it has any privileged access. Every state-changing method is gated twice —
//! by a component access rule naming the party's public key, and by an explicit signer
//! check inside the method — so the two-party rules cannot be bypassed.

use tari_template_lib::prelude::*;

#[template]
mod rent_vault {
    use super::*;

    /// A single rental deposit agreement between one renter and one owner.
    ///
    /// NOTE: this struct must stay the first type declared in the module — the
    /// `#[template]` macro takes the first struct/enum it finds as the template name.
    pub struct RentVault {
        /// Holds the locked deposit. Only ever holds the native TARI token.
        vault: Vault,
        /// The account that receives refunds and claim remainders.
        renter_account: ComponentAddress,
        /// The account that receives a paid claim.
        owner_account: ComponentAddress,
        /// Public key of the renter, who creates the agreement and locks the deposit.
        renter_key: RistrettoPublicKeyBytes,
        /// Public key of the owner (counterparty) who may claim after the rental ends.
        owner_key: RistrettoPublicKeyBytes,
        /// A short description of the rented item, e.g. "Cinema camera kit".
        item: String,
        /// The exact deposit the renter must lock.
        deposit_amount: Amount,
        /// The epoch at which the rental ends and claims open.
        rental_end_epoch: u64,
        /// How many epochs a filed claim stays disputable before it can be executed.
        dispute_window_epochs: u64,
        /// How many epochs after the rental ends the owner has to file a claim before
        /// the deposit becomes auto-refundable.
        claim_window_epochs: u64,
        /// Current lifecycle stage.
        status: Status,
        /// How the agreement settled, once it has.
        outcome: Option<Outcome>,
        /// Whether the renter has confirmed the return of the item.
        renter_confirmed: bool,
        /// Whether the owner has confirmed the return of the item.
        owner_confirmed: bool,
        /// The amount of the pending claim, if any.
        claimed_amount: Amount,
        /// The epoch at which the pending claim was filed.
        claim_epoch: u64,
    }

    /// The lifecycle stage of a deposit agreement.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Status {
        /// The agreement exists but the renter has not locked the deposit yet.
        AwaitingDeposit,
        /// The deposit is locked in the vault and the rental is running.
        Funded,
        /// The owner has filed a claim; the dispute window is running.
        ClaimPending,
        /// The deposit has been paid out and the agreement is final.
        Settled,
    }

    /// How a settled agreement was resolved.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Outcome {
        /// Both parties confirmed the return; the renter was refunded in full.
        MutualRelease,
        /// A claim survived its dispute window; the owner was paid the claimed amount
        /// and the renter was refunded the remainder.
        ClaimPaid,
        /// The renter disputed the claim in time; the claim was voided and the renter
        /// was refunded in full.
        DisputeRefund,
        /// The owner never claimed; the renter was refunded in full.
        AutoRefund,
    }

    impl RentVault {
        /// Creates a new deposit agreement. The transaction signer becomes the renter.
        ///
        /// The agreement starts in `AwaitingDeposit`; no funds move until the renter
        /// calls [`RentVault::deposit`] with exactly `deposit_amount`.
        ///
        /// * `item` — a short description of the rented item.
        /// * `renter_account` — the signer's account; receives all renter payouts.
        /// * `owner_account` — the counterparty's account; receives a paid claim.
        /// * `owner_key` — the counterparty's public key; only they can file a claim.
        /// * `rental_end_epoch` — must be in the future.
        /// * `dispute_window_epochs` / `claim_window_epochs` — must both be at least 1.
        // A template constructor's parameters are the transaction's arguments, so they
        // cannot be grouped into a config struct the way ordinary Rust APIs would.
        #[allow(clippy::too_many_arguments)]
        pub fn new(
            item: String,
            renter_account: ComponentAddress,
            owner_account: ComponentAddress,
            owner_key: RistrettoPublicKeyBytes,
            deposit_amount: Amount,
            rental_end_epoch: u64,
            dispute_window_epochs: u64,
            claim_window_epochs: u64,
        ) -> Component<Self> {
            // The creator of the agreement is the renter; there is no way to create an
            // agreement "for" somebody else.
            let renter_key = CallerContext::transaction_signer_public_key();

            assert!(
                deposit_amount.is_positive(),
                "Deposit amount must be positive"
            );
            assert!(
                rental_end_epoch > Consensus::current_epoch(),
                "Rental end epoch must be in the future"
            );
            assert!(
                dispute_window_epochs >= 1,
                "Dispute window must be at least one epoch"
            );
            assert!(
                claim_window_epochs >= 1,
                "Claim window must be at least one epoch"
            );
            assert!(
                renter_account != owner_account,
                "Renter and owner must be different accounts"
            );
            assert!(
                renter_key != owner_key,
                "Renter and owner must be different parties"
            );

            // Each state-changing method names the one party allowed to call it (or
            // both parties, for `confirm_return`). Settlement triggers that only move
            // funds to their pre-agreed destination are open to anyone, so a deposit
            // can never be stranded by an absent counterparty. Everything else —
            // including any method added later — is denied by default.
            let access_rules = ComponentAccessRules::new()
                .method("deposit", rule!(public_key(renter_key)))
                .method(
                    "confirm_return",
                    rule!(any_of(public_key(renter_key), public_key(owner_key))),
                )
                .method("file_claim", rule!(public_key(owner_key)))
                .method("dispute_claim", rule!(public_key(renter_key)))
                .method("execute_claim", rule!(allow_all))
                .method("trigger_refund", rule!(allow_all))
                .method("status", rule!(allow_all))
                .method("outcome", rule!(allow_all))
                .method("item", rule!(allow_all))
                .method("deposit_amount", rule!(allow_all))
                .method("claimed_amount", rule!(allow_all))
                .method("vault_balance", rule!(allow_all))
                .method("rental_end_epoch", rule!(allow_all))
                .default(rule!(deny_all));

            emit_event(
                "rent_vault.agreement_created",
                metadata![
                    "item" => item.clone(),
                    "deposit_amount" => deposit_amount.to_string(),
                    "rental_end_epoch" => rental_end_epoch.to_string(),
                ],
            );

            Component::new(Self {
                vault: Vault::new_empty(TARI_TOKEN),
                renter_account,
                owner_account,
                renter_key,
                owner_key,
                item,
                deposit_amount,
                rental_end_epoch,
                dispute_window_epochs,
                claim_window_epochs,
                status: Status::AwaitingDeposit,
                outcome: None,
                renter_confirmed: false,
                owner_confirmed: false,
                claimed_amount: Amount::zero(),
                claim_epoch: 0,
            })
            .with_access_rules(access_rules)
            // No owner: only the access rules above govern this component. The creator
            // gets no implicit privileges, and the rules are locked at creation.
            .with_owner_rule(OwnerRule::None)
            .create()
        }

        /// Locks the deposit. Renter only.
        ///
        /// `payment` must be exactly the agreed `deposit_amount` in the native TARI
        /// token; anything else is rejected so funds can never be locked in the wrong
        /// amount or the wrong asset.
        pub fn deposit(&mut self, payment: Bucket) {
            self.assert_signer_is_renter("Only the renter can deposit the agreed amount");
            assert!(
                matches!(self.status, Status::AwaitingDeposit),
                "The deposit has already been made"
            );
            assert!(
                payment.resource_address() == TARI_TOKEN,
                "The deposit must be paid in the native TARI token"
            );
            assert!(
                payment.amount() == self.deposit_amount,
                "Deposit must be exactly the agreed amount"
            );

            self.vault.deposit(payment);
            self.status = Status::Funded;

            emit_event(
                "rent_vault.deposit_locked",
                metadata!["amount" => self.deposit_amount.to_string()],
            );
        }

        /// Confirms that the rented item was returned in good condition.
        ///
        /// Either party may confirm, in either order. When both have confirmed, the
        /// deposit is refunded to the renter in full and the agreement settles.
        pub fn confirm_return(&mut self) {
            assert!(
                matches!(self.status, Status::Funded),
                "Returns can only be confirmed while the agreement is funded"
            );

            let signer = CallerContext::transaction_signer_public_key();
            if signer == self.renter_key {
                assert!(
                    !self.renter_confirmed,
                    "The renter has already confirmed the return"
                );
                self.renter_confirmed = true;
                emit_event(
                    "rent_vault.return_confirmed",
                    metadata!["party" => "renter".to_string()],
                );
            } else if signer == self.owner_key {
                assert!(
                    !self.owner_confirmed,
                    "The owner has already confirmed the return"
                );
                self.owner_confirmed = true;
                emit_event(
                    "rent_vault.return_confirmed",
                    metadata!["party" => "owner".to_string()],
                );
            } else {
                panic!("Only the renter or the owner can confirm the return");
            }

            if self.renter_confirmed && self.owner_confirmed {
                self.refund_renter_in_full(Outcome::MutualRelease);
            }
        }

        /// Files a claim against the deposit, e.g. for damage to the rented item.
        /// Owner only, and only once the rental has ended.
        ///
        /// The claim does not pay out immediately: it opens the dispute window, during
        /// which the renter may void it with [`RentVault::dispute_claim`]. After the
        /// window, anyone may settle it with [`RentVault::execute_claim`].
        pub fn file_claim(&mut self, amount: Amount) {
            self.assert_signer_is_owner("Only the owner can file a claim");
            assert!(
                matches!(self.status, Status::Funded),
                "Claims can only be filed on a funded agreement"
            );
            assert!(
                Consensus::current_epoch() >= self.rental_end_epoch,
                "The rental has not ended yet"
            );
            assert!(
                amount.is_positive(),
                "A claim must be for a positive amount"
            );
            assert!(
                amount <= self.deposit_amount,
                "A claim cannot exceed the deposit"
            );

            self.claimed_amount = amount;
            self.claim_epoch = Consensus::current_epoch();
            self.status = Status::ClaimPending;

            emit_event(
                "rent_vault.claim_filed",
                metadata![
                    "amount" => amount.to_string(),
                    "dispute_deadline" =>
                        (self.claim_epoch + self.dispute_window_epochs).to_string(),
                ],
            );
        }

        /// Disputes the pending claim. Renter only, and only during the dispute window.
        ///
        /// Disputing voids the claim and refunds the deposit to the renter in full.
        /// RentVault is a custody layer, not a court: the claim and the dispute are both
        /// on the public record, and any further argument happens off-chain.
        pub fn dispute_claim(&mut self) {
            self.assert_signer_is_renter("Only the renter can dispute a claim");
            assert!(
                matches!(self.status, Status::ClaimPending),
                "There is no pending claim to dispute"
            );
            assert!(
                Consensus::current_epoch() <= self.claim_epoch + self.dispute_window_epochs,
                "The dispute window has closed"
            );

            emit_event(
                "rent_vault.claim_disputed",
                metadata!["amount" => self.claimed_amount.to_string()],
            );
            self.refund_renter_in_full(Outcome::DisputeRefund);
        }

        /// Settles a claim whose dispute window has passed undisputed. Anyone may call
        /// this: the owner receives the claimed amount and the renter is refunded the
        /// remainder of the deposit.
        pub fn execute_claim(&mut self) {
            assert!(
                matches!(self.status, Status::ClaimPending),
                "There is no pending claim to execute"
            );
            assert!(
                Consensus::current_epoch() > self.claim_epoch + self.dispute_window_epochs,
                "The dispute window is still open"
            );

            let claimed = self.claimed_amount;
            let claim_bucket = self.vault.withdraw(claimed);
            Self::pay_to(self.owner_account, claim_bucket);

            let remainder = self.vault.balance();
            if remainder.is_positive() {
                let remainder_bucket = self.vault.withdraw_all();
                Self::pay_to(self.renter_account, remainder_bucket);
            }

            self.status = Status::Settled;
            self.outcome = Some(Outcome::ClaimPaid);

            emit_event(
                "rent_vault.settled",
                metadata![
                    "outcome" => "ClaimPaid".to_string(),
                    "to_owner" => claimed.to_string(),
                    "to_renter" => remainder.to_string(),
                ],
            );
        }

        /// Refunds the deposit in full when the owner never filed a claim within the
        /// claim window after the rental ended. Anyone may call this, so the renter
        /// never has to chase the owner — or depend on the owner existing at all — to
        /// get their deposit back.
        pub fn trigger_refund(&mut self) {
            assert!(
                matches!(self.status, Status::Funded),
                "Auto-refund only applies to a funded agreement with no claim"
            );
            assert!(
                Consensus::current_epoch() > self.rental_end_epoch + self.claim_window_epochs,
                "The owner can still claim"
            );

            self.refund_renter_in_full(Outcome::AutoRefund);
        }

        /// The current lifecycle stage: `AwaitingDeposit`, `Funded`, `ClaimPending` or
        /// `Settled`.
        pub fn status(&self) -> String {
            match self.status {
                Status::AwaitingDeposit => "AwaitingDeposit",
                Status::Funded => "Funded",
                Status::ClaimPending => "ClaimPending",
                Status::Settled => "Settled",
            }
            .to_string()
        }

        /// How the agreement settled, or `None` while it is still open.
        pub fn outcome(&self) -> String {
            match self.outcome {
                None => "None",
                Some(Outcome::MutualRelease) => "MutualRelease",
                Some(Outcome::ClaimPaid) => "ClaimPaid",
                Some(Outcome::DisputeRefund) => "DisputeRefund",
                Some(Outcome::AutoRefund) => "AutoRefund",
            }
            .to_string()
        }

        /// The description of the rented item given at creation.
        pub fn item(&self) -> String {
            self.item.clone()
        }

        /// The agreed deposit amount.
        pub fn deposit_amount(&self) -> Amount {
            self.deposit_amount
        }

        /// The amount of the owner's claim, or zero if no claim was filed.
        pub fn claimed_amount(&self) -> Amount {
            self.claimed_amount
        }

        /// The amount currently locked in the vault.
        pub fn vault_balance(&self) -> Amount {
            self.vault.balance()
        }

        /// The epoch at which the rental ends and claims open.
        pub fn rental_end_epoch(&self) -> u64 {
            self.rental_end_epoch
        }

        /// Panics unless the transaction signer is the renter.
        ///
        /// This duplicates the method access rules on purpose: the signer check is the
        /// semantic gate for the two-party rules and keeps them intact even if the
        /// component's access rules are ever reconfigured.
        fn assert_signer_is_renter(&self, message: &str) {
            assert!(
                CallerContext::transaction_signer_public_key() == self.renter_key,
                "{}",
                message
            );
        }

        /// Panics unless the transaction signer is the owner.
        fn assert_signer_is_owner(&self, message: &str) {
            assert!(
                CallerContext::transaction_signer_public_key() == self.owner_key,
                "{}",
                message
            );
        }

        /// Pays a bucket into an account component.
        fn pay_to(account: ComponentAddress, bucket: Bucket) {
            ComponentManager::get(account).invoke("deposit", args![bucket]);
        }

        /// Refunds the whole vault balance to the renter and settles the agreement.
        fn refund_renter_in_full(&mut self, outcome: Outcome) {
            let amount = self.vault.balance();
            let funds = self.vault.withdraw_all();
            Self::pay_to(self.renter_account, funds);
            self.status = Status::Settled;
            self.outcome = Some(outcome);

            let outcome_name = match outcome {
                Outcome::MutualRelease => "MutualRelease",
                Outcome::ClaimPaid => "ClaimPaid",
                Outcome::DisputeRefund => "DisputeRefund",
                Outcome::AutoRefund => "AutoRefund",
            };
            emit_event(
                "rent_vault.settled",
                metadata![
                    "outcome" => outcome_name.to_string(),
                    "to_renter" => amount.to_string(),
                ],
            );
        }
    }
}
