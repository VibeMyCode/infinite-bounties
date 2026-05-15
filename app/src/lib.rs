#![no_std]

use sails_rs::{
    cell::RefCell,
    collections::BTreeMap,
    gstd::{exec, msg},
    prelude::*,
};

// ─── State ─────────────────────────────────────────────────────────────────

#[derive(Encode, Decode, TypeInfo, Clone)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub struct Bounty {
    pub id: u64,
    pub creator: ActorId,
    pub description: String,
    pub metadata_url: String,
    pub reward: u128,
    pub status: BountyStatus,
    pub claimant: Option<ActorId>,
    pub proof_url: Option<String>,
    pub created_at: u64,
}

#[derive(Encode, Decode, TypeInfo, Clone, PartialEq)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub enum BountyStatus {
    Open,
    Claimed,
    Submitted,
    Approved,
    Cancelled,
}

#[derive(Encode, Decode, TypeInfo, Clone)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub struct Config {
    pub admin: ActorId,
    pub fee: u128,
    pub bounty_count: u64,
}

#[derive(Encode, Decode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub struct MigrationData {
    pub admin: ActorId,
    pub fee: u128,
    pub next_bounty_id: u64,
    pub bounties: Vec<Bounty>,
}

#[derive(Clone)]
struct State {
    admin: ActorId,
    fee: u128,
    next_bounty_id: u64,
    bounties: BTreeMap<u64, Bounty>,
}

// ─── Events ─────────────────────────────────────────────────────────────────

#[sails_rs::event]
#[derive(Encode, Decode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub enum Event {
    BountyPosted {
        id: u64,
        creator: ActorId,
        description: String,
        reward: u128,
        fee_paid: u128,
    },
    BountyClaimed {
        id: u64,
        claimant: ActorId,
    },
    WorkSubmitted {
        id: u64,
        proof_url: String,
    },
    BountyApproved {
        id: u64,
        reward: u128,
        creator: ActorId,
        claimant: ActorId,
    },
    BountyCancelled {
        id: u64,
        reward_returned: u128,
    },
    FeeUpdated {
        old_fee: u128,
        new_fee: u128,
    },
    FeesWithdrawn {
        amount: u128,
        to: ActorId,
    },
}

// ─── Service ────────────────────────────────────────────────────────────────

pub struct BountyBoardService<'a> {
    state: &'a RefCell<State>,
}

#[derive(Encode, Decode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub struct BountyPage {
    pub bounties: Vec<Bounty>,
}

impl<'a> BountyBoardService<'a> {
    pub(crate) fn new(state: &'a RefCell<State>) -> Self {
        Self { state }
    }

    fn ensure_admin(&self) -> Result<(), &'static str> {
        let caller = msg::source();
        let state = self.state.borrow();
        if caller != state.admin {
            return Err("NotAuthorized");
        }
        Ok(())
    }

    fn ensure_min_value(&self, min: u128) -> Result<u128, &'static str> {
        let value = msg::value();
        if value < min {
            return Err("ValueTooLow");
        }
        Ok(value)
    }
}

#[service(events = Event)]
impl BountyBoardService<'_> {
    // ─── Commands ──────────────────────────────────────────────────────────

    /// Post a new bounty. Requires msg::value() >= fee + reward.
    /// fee is collected by the contract, reward is locked for the claimant.
    #[export(unwrap_result)]
    pub fn post_bounty(
        &mut self,
        description: String,
        metadata_url: String,
    ) -> Result<u64, &'static str> {
        if description.is_empty() {
            return Err("DescriptionEmpty");
        }

        let fee;
        let next_id;
        {
            let state = self.state.borrow();
            fee = state.fee;
            next_id = state.next_bounty_id;
        }

        let value = self.ensure_min_value(fee)?;
        let reward = value - fee;
        if reward == 0 {
            return Err("RewardZero");
        }

        let caller = msg::source();
        let now = exec::block_timestamp();

        let mut state = self.state.borrow_mut();
        state.bounties.insert(
            next_id,
            Bounty {
                id: next_id,
                creator: caller,
                description: description.clone(),
                metadata_url,
                reward,
                status: BountyStatus::Open,
                claimant: None,
                proof_url: None,
                created_at: now,
            },
        );
        state.next_bounty_id += 1;
        drop(state);

        let _ = self.emit_event(Event::BountyPosted {
            id: next_id,
            creator: caller,
            description,
            reward,
            fee_paid: fee,
        });

        Ok(next_id)
    }

    /// Claim an open bounty. Caller becomes the claimant.
    #[export(unwrap_result)]
    pub fn claim_bounty(&mut self, bounty_id: u64) -> Result<(), &'static str> {
        let caller = msg::source();
        let mut state = self.state.borrow_mut();
        let bounty = state
            .bounties
            .get_mut(&bounty_id)
            .ok_or("BountyNotFound")?;

        if bounty.status != BountyStatus::Open {
            return Err("NotOpen");
        }
        if bounty.creator == caller {
            return Err("CannotClaimOwnBounty");
        }

        bounty.status = BountyStatus::Claimed;
        bounty.claimant = Some(caller);
        drop(state);

        let _ = self.emit_event(Event::BountyClaimed {
            id: bounty_id,
            claimant: caller,
        });

        Ok(())
    }

    /// Submit proof of work for a claimed bounty.
    #[export(unwrap_result)]
    pub fn submit_work(
        &mut self,
        bounty_id: u64,
        proof_url: String,
    ) -> Result<(), &'static str> {
        let caller = msg::source();
        let mut state = self.state.borrow_mut();
        let bounty = state
            .bounties
            .get_mut(&bounty_id)
            .ok_or("BountyNotFound")?;

        if bounty.status != BountyStatus::Claimed {
            return Err("NotClaimed");
        }
        if bounty.claimant != Some(caller) {
            return Err("NotClaimant");
        }

        bounty.status = BountyStatus::Submitted;
        bounty.proof_url = Some(proof_url.clone());
        drop(state);

        let _ = self.emit_event(Event::WorkSubmitted {
            id: bounty_id,
            proof_url,
        });

        Ok(())
    }

    /// Approve submitted work. Releases the reward to the claimant.
    #[export(unwrap_result)]
    pub fn approve_bounty(&mut self, bounty_id: u64) -> Result<(), &'static str> {
        let caller = msg::source();
        let mut state = self.state.borrow_mut();
        let bounty = state
            .bounties
            .get_mut(&bounty_id)
            .ok_or("BountyNotFound")?;

        if bounty.status != BountyStatus::Submitted {
            return Err("NotSubmitted");
        }
        if bounty.creator != caller {
            return Err("NotCreator");
        }

        let claimant = bounty.claimant.ok_or("NoClaimant")?;
        let reward = bounty.reward;
        bounty.status = BountyStatus::Approved;

        // Send reward to claimant (0 gas — simple value transfer to wallet)
        msg::send_with_gas(claimant, (), 0, reward).expect("send reward failed");
        drop(state);

        let _ = self.emit_event(Event::BountyApproved {
            id: bounty_id,
            reward,
            creator: caller,
            claimant,
        });

        Ok(())
    }

    /// Cancel an open (unclaimed) bounty. Reward is returned to creator.
    #[export(unwrap_result)]
    pub fn cancel_bounty(&mut self, bounty_id: u64) -> Result<(), &'static str> {
        let caller = msg::source();
        let mut state = self.state.borrow_mut();
        let bounty = state
            .bounties
            .get_mut(&bounty_id)
            .ok_or("BountyNotFound")?;

        if bounty.status != BountyStatus::Open {
            return Err("NotOpen");
        }
        if bounty.creator != caller {
            return Err("NotCreator");
        }

        let reward = bounty.reward;
        bounty.status = BountyStatus::Cancelled;

        // Return reward to creator (0 gas — simple value transfer)
        msg::send_with_gas(caller, (), 0, reward).expect("send refund failed");
        drop(state);

        let _ = self.emit_event(Event::BountyCancelled {
            id: bounty_id,
            reward_returned: reward,
        });

        Ok(())
    }

    // ─── Admin commands ────────────────────────────────────────────────────

    /// Set the service fee. Admin only.
    #[export(unwrap_result)]
    pub fn set_fee(&mut self, new_fee: u128) -> Result<(), &'static str> {
        self.ensure_admin()?;
        let mut state = self.state.borrow_mut();
        state.fee = new_fee;
        drop(state);

        let _ = self.emit_event(Event::FeeUpdated {
            old_fee: 0,
            new_fee,
        });

        Ok(())
    }

    /// Withdraw accumulated fees to admin. Admin only.
    #[export(unwrap_result)]
    pub fn withdraw_fees(&mut self) -> Result<(), &'static str> {
        self.ensure_admin()?;
        let admin = {
            let state = self.state.borrow();
            state.admin
        };

        // Send entire program balance to admin
        // (locked rewards in active bounties will remain)
        let balance = exec::value_available();
        if balance == 0 {
            return Err("NoFeesToWithdraw");
        }

        msg::send_with_gas(admin, (), 0, balance).expect("send fees failed");

        let _ = self.emit_event(Event::FeesWithdrawn {
            amount: balance,
            to: admin,
        });

        Ok(())
    }

    /// Export all state for migration to a new program. Admin only.
    #[export(unwrap_result)]
    pub fn export_state(&self) -> Result<Vec<u8>, &'static str> {
        self.ensure_admin()?;
        let state = self.state.borrow();
        let migration = MigrationData {
            admin: state.admin,
            fee: state.fee,
            next_bounty_id: state.next_bounty_id,
            bounties: state.bounties.values().cloned().collect(),
        };
        let encoded = migration.encode();
        Ok(encoded)
    }

    /// Import state from a previous program. Admin only.
    /// Overwrites ALL current state with the imported data.
    #[export(unwrap_result)]
    pub fn import_state(&mut self, encoded_state: Vec<u8>) -> Result<(), &'static str> {
        self.ensure_admin()?;
        let migration =
            MigrationData::decode(&mut &encoded_state[..]).map_err(|_| "InvalidMigrationData")?;

        let mut state = self.state.borrow_mut();
        state.admin = migration.admin;
        state.fee = migration.fee;
        state.next_bounty_id = migration.next_bounty_id;
        state.bounties.clear();
        for bounty in migration.bounties {
            state.bounties.insert(bounty.id, bounty);
        }
        drop(state);

        Ok(())
    }

    // ─── Queries ───────────────────────────────────────────────────────────

    /// Get bounty details by ID.
    #[export]
    pub fn get_bounty(&self, id: u64) -> Option<Bounty> {
        let state = self.state.borrow();
        state.bounties.get(&id).cloned()
    }

    /// Get all bounties created by a specific wallet, with pagination.
    #[export]
    pub fn get_bounties_by_creator(
        &self,
        creator: ActorId,
        cursor: Option<u64>,
        limit: u32,
    ) -> BountyPage {
        let state = self.state.borrow();
        let mut result = Vec::new();
        let skip_before = cursor.unwrap_or(0);
        for bounty in state.bounties.values() {
            if bounty.creator == creator && bounty.id > skip_before {
                result.push(bounty.clone());
                if result.len() >= limit as usize {
                    break;
                }
            }
        }
        BountyPage { bounties: result }
    }

    /// Get all bounties in a given status, with pagination.
    #[export]
    pub fn get_bounties_by_status(
        &self,
        status: BountyStatus,
        cursor: Option<u64>,
        limit: u32,
    ) -> BountyPage {
        let state = self.state.borrow();
        let mut result = Vec::new();
        let skip_before = cursor.unwrap_or(0);
        for bounty in state.bounties.values() {
            if bounty.status == status && bounty.id > skip_before {
                result.push(bounty.clone());
                if result.len() >= limit as usize {
                    break;
                }
            }
        }
        BountyPage { bounties: result }
    }

    /// Get service config (admin, fee, bounty count).
    #[export]
    pub fn get_config(&self) -> Config {
        let state = self.state.borrow();
        Config {
            admin: state.admin,
            fee: state.fee,
            bounty_count: state.next_bounty_id - 1,
        }
    }
}

// ─── Program ────────────────────────────────────────────────────────────────

pub struct Program {
    state: RefCell<State>,
}

#[sails_rs::program]
impl Program {
    pub fn new(admin: ActorId, fee: u128) -> Self {
        Self {
            state: RefCell::new(State {
                admin,
                fee,
                next_bounty_id: 1,
                bounties: BTreeMap::new(),
            }),
        }
    }

    pub fn bounty_board(&self) -> BountyBoardService<'_> {
        BountyBoardService::new(&self.state)
    }
}
