//! Stactor tests - determinism and property-based.
//! These tests use the sync API, so they're disabled when async feature is enabled.

#![cfg(not(feature = "async"))]

use proptest::prelude::*;
use ssm::state_machine;

// =============================================================================
// State Machines
// =============================================================================

state_machine! {
    pub enum Counter {
        #[initial]
        Ready,
        Working,
    }

    impl Counter {
        fn start(self: Ready) -> Working {
            CounterStateData::Working
        }

        fn done(self: Working) -> Ready {
            CounterStateData::Ready
        }
    }
}

// =============================================================================
// Bank Account - demonstrates Result<State, Error> for application errors
// =============================================================================

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BankContext {
    balance: i64,
}

impl Default for BankContext {
    fn default() -> Self {
        Self { balance: 0 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BankError {
    InsufficientFunds { available: i64, requested: i64 },
    InvalidAmount,
}

state_machine! {
    pub enum BankAccount {
        #[initial]
        Idle,
        Processing,
    }

    type Context = BankContext;

    impl BankAccount {
        fn begin_transaction(self: Idle) -> Processing {
            BankAccountStateData::Processing
        }

        // Deposit always succeeds (unless invalid amount)
        fn deposit(self: Processing, amount: i64) -> Result<Idle, BankError> {
            if amount <= 0 {
                Err(BankError::InvalidAmount)
            } else {
                ctx.balance += amount;
                Ok(BankAccountStateData::Idle)
            }
        }

        // Withdraw can fail if insufficient funds
        fn withdraw(self: Processing, amount: i64) -> Result<Idle, BankError> {
            if amount <= 0 {
                Err(BankError::InvalidAmount)
            } else if ctx.balance < amount {
                Err(BankError::InsufficientFunds {
                    available: ctx.balance,
                    requested: amount,
                })
            } else {
                ctx.balance -= amount;
                Ok(BankAccountStateData::Idle)
            }
        }

        // Cancel returns to idle without error
        fn cancel(self: Processing) -> Idle {
            BankAccountStateData::Idle
        }
    }
}

state_machine! {
    pub enum Msg {
        #[initial]
        Pending,
        Claimed { leader: u64, term: u64 },
        Done,
    }

    impl Msg {
        fn claim(self: Pending, leader: u64, term: u64) -> Claimed {
            MsgStateData::Claimed { leader, term }
        }

        // For single-source, fields are extracted as locals (leader, term)
        #[guard(leader == check_leader && term == check_term)]
        fn finish(self: Claimed, check_leader: u64, check_term: u64) -> Done {
            MsgStateData::Done
        }

        fn release(self: Claimed) -> Pending {
            MsgStateData::Pending
        }
    }
}

// =============================================================================
// Determinism Tests
// =============================================================================

#[test]
fn test_counter_determinism() {
    for _ in 0..5 {
        let c = Counter::new();
        c.start().unwrap();
        c.done().unwrap();
        c.start().unwrap();
        c.done().unwrap();
    }
}

#[test]
fn test_guard_determinism() {
    for _ in 0..5 {
        let m = Msg::new();
        m.claim(1, 100).unwrap();

        // Wrong - should fail
        assert!(m.finish(2, 100).is_err());

        // Correct - should succeed
        m.finish(1, 100).unwrap();
    }
}

#[test]
fn test_sequential_ops() {
    for _ in 0..5 {
        let m = Msg::new();
        m.claim(1, 100).unwrap();
        m.release().unwrap();
        m.claim(2, 200).unwrap();
        m.finish(2, 200).unwrap();
    }
}

// =============================================================================
// Property Tests (proptest)
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    #[test]
    fn guard_only_passes_with_matching_credentials(
        claim_leader in 1u64..=5,
        claim_term in 1u64..=10,
        finish_leader in 1u64..=5,
        finish_term in 1u64..=10,
    ) {
        let m = Msg::new();
        m.claim(claim_leader, claim_term).unwrap();
        let result = m.finish(finish_leader, finish_term);

        let should_pass = claim_leader == finish_leader && claim_term == finish_term;
        prop_assert_eq!(
            result.is_ok(),
            should_pass,
            "finish({},{}) after claim({},{}) should {} but {}",
            finish_leader, finish_term, claim_leader, claim_term,
            if should_pass { "pass" } else { "fail" },
            if result.is_ok() { "passed" } else { "failed" }
        );
    }

    #[test]
    fn release_allows_reclaim(
        l1 in 1u64..=10,
        t1 in 1u64..=100,
        l2 in 1u64..=10,
        t2 in 1u64..=100,
    ) {
        let m = Msg::new();
        m.claim(l1, t1).unwrap();
        m.release().unwrap();
        m.claim(l2, t2).unwrap();
        m.finish(l2, t2).unwrap();
    }
}

// =============================================================================
// Unit Tests
// =============================================================================

#[test]
fn test_guard_failure() {
    let m = Msg::new();
    m.claim(1, 100).unwrap();

    let err = m.finish(999, 999).unwrap_err();
    assert!(matches!(err, FinishError::GuardFailed));

    // Can still use handle - retry with correct values
    m.finish(1, 100).unwrap();
}

#[test]
fn test_basic_flow() {
    let c = Counter::new();
    c.start().unwrap();
    c.done().unwrap();
    c.start().unwrap();
    c.done().unwrap();
}

// =============================================================================
// Bank Account Tests - Result<State, Error> return type
// =============================================================================

#[test]
fn test_bank_deposit_success() {
    let bank = BankAccount::new();
    bank.begin_transaction().unwrap();
    bank.deposit(100).unwrap();
    bank.begin_transaction().unwrap();
    bank.deposit(50).unwrap();
    // Balance should be 150
}

#[test]
fn test_bank_withdraw_success() {
    let bank = BankAccount::new();

    // Deposit first
    bank.begin_transaction().unwrap();
    bank.deposit(100).unwrap();

    // Then withdraw
    bank.begin_transaction().unwrap();
    bank.withdraw(50).unwrap();
    // Balance should be 50
}

#[test]
fn test_bank_withdraw_insufficient_funds() {
    let bank = BankAccount::new();

    // Try to withdraw without depositing
    bank.begin_transaction().unwrap();
    let err = bank.withdraw(100).unwrap_err();
    assert_eq!(err, ssm::TransitionError::User(BankError::InsufficientFunds { available: 0, requested: 100 }));

    // Can cancel and recover
    bank.cancel().unwrap();
}

#[test]
fn test_bank_invalid_amount() {
    let bank = BankAccount::new();
    bank.begin_transaction().unwrap();

    // Try negative deposit
    let err = bank.deposit(-50).unwrap_err();
    assert_eq!(err, ssm::TransitionError::User(BankError::InvalidAmount));

    // Can still cancel
    bank.cancel().unwrap();
}

// =============================================================================
// State Inspection Tests - get_state() and snapshot()
// =============================================================================

#[test]
fn test_get_state() {
    let m = Msg::new();

    // Initial state
    let state = m.get_state();
    assert!(matches!(state, MsgStateData::Pending));

    // After claim
    m.claim(42, 100).unwrap();
    let state = m.get_state();
    assert!(matches!(state, MsgStateData::Claimed { leader: 42, term: 100 }));

    // After finish
    m.finish(42, 100).unwrap();
    let state = m.get_state();
    assert!(matches!(state, MsgStateData::Done));
}

#[test]
fn test_snapshot_for_serialization() {
    let m = Msg::new();
    m.claim(1, 50).unwrap();

    // Get snapshot
    let snapshot = m.snapshot();

    // Serialize to JSON
    let json = serde_json::to_string(&snapshot).unwrap();
    assert!(json.contains("Claimed"));
    assert!(json.contains("\"leader\":1"));
    assert!(json.contains("\"term\":50"));

    // Deserialize back
    let restored: MsgSnapshot = serde_json::from_str(&json).unwrap();
    assert!(matches!(restored.state, MsgStateData::Claimed { leader: 1, term: 50 }));
}

// =============================================================================
// Concurrent Access Tests
// =============================================================================

#[test]
fn test_clone_concurrent_access() {
    let c1 = Counter::new();
    let c2 = c1.clone();

    // Both can interact with the same state
    c1.start().unwrap();

    // c2 sees the state change
    let state = c2.get_state();
    assert!(matches!(state, CounterStateData::Working));

    // c2 can continue
    c2.done().unwrap();

    // c1 sees final state
    let state = c1.get_state();
    assert!(matches!(state, CounterStateData::Ready));
}

#[test]
fn test_invalid_state_error() {
    let c = Counter::new();

    // Can't call done() when in Ready state
    let err = c.done().unwrap_err();
    assert_eq!(err, DoneError::InvalidState);

    // Start works
    c.start().unwrap();

    // Now done() works
    c.done().unwrap();

    // Can call start() again because we're back in Ready
    c.start().unwrap();
}

// =============================================================================
// Lifecycle Hooks Tests
// =============================================================================

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct HookContext {
    pub enter_count: u32,
    pub exit_count: u32,
    pub log: Vec<String>,
}

state_machine! {
    pub enum HookMachine {
        #[initial]
        Idle,
        Active { value: u32 },
        Done,
    }

    type Context = HookContext;

    impl HookMachine {
        fn activate(self: Idle, value: u32) -> Active {
            HookMachineStateData::Active { value }
        }

        fn update(self: Active, new_value: u32) -> Active {
            HookMachineStateData::Active { value: new_value }
        }

        fn complete(self: Active) -> Done {
            HookMachineStateData::Done
        }

        fn reset(self: Done) -> Idle {
            HookMachineStateData::Idle
        }

        #[on_enter(Active)]
        fn entering_active(&mut self) {
            ctx.enter_count += 1;
            ctx.log.push(format!("entered Active with value={}", value));
        }

        #[on_exit(Active)]
        fn exiting_active(&mut self) {
            ctx.exit_count += 1;
            ctx.log.push(format!("exited Active with value={}", value));
        }
    }
}

#[test]
fn test_lifecycle_hooks_basic() {
    let m = HookMachine::new();

    // Activate: should trigger on_enter(Active)
    m.activate(42).unwrap();

    let snapshot = m.snapshot();
    assert_eq!(snapshot.context.enter_count, 1);
    assert_eq!(snapshot.context.exit_count, 0);
    assert_eq!(snapshot.context.log, vec!["entered Active with value=42"]);
}

#[test]
fn test_lifecycle_hooks_exit_and_enter() {
    let m = HookMachine::new();

    // Activate: on_enter
    m.activate(10).unwrap();

    // Update: on_exit(Active), then on_enter(Active)
    m.update(20).unwrap();

    let snapshot = m.snapshot();
    assert_eq!(snapshot.context.enter_count, 2);
    assert_eq!(snapshot.context.exit_count, 1);
    assert_eq!(snapshot.context.log, vec![
        "entered Active with value=10",
        "exited Active with value=10",
        "entered Active with value=20",
    ]);
}

#[test]
fn test_lifecycle_hooks_complete_flow() {
    let m = HookMachine::new();

    // Idle -> Active (enter)
    m.activate(5).unwrap();

    // Active -> Done (exit Active, no hooks on Done)
    m.complete().unwrap();

    let snapshot = m.snapshot();
    assert_eq!(snapshot.context.enter_count, 1);
    assert_eq!(snapshot.context.exit_count, 1);
    assert_eq!(snapshot.context.log, vec![
        "entered Active with value=5",
        "exited Active with value=5",
    ]);
}
