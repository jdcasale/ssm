//! Resilient-relay MessageState implemented with stactor.
//!
//! This is the full implementation of the message delivery state machine
//! for exactly-once semantics in a Raft-based relay system.

use ssm::state_machine;

pub type NodeId = u64;

/// Reason for dead-lettering a message.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum DeadLetterReason {
    AmbiguousFailure { reason: String },
    MaxRetriesExceeded,
}

/// Errors that can occur during message state transitions.
#[derive(Debug, Clone, PartialEq)]
pub enum MessageError {
    /// Tried to mark in-doubt but leader/term hasn't changed
    SameLeader,
    /// Tried to ack/nack with wrong leader/term credentials
    LeaderMismatch {
        expected_leader: NodeId,
        expected_term: u64,
        got_leader: NodeId,
        got_term: u64,
    },
}

/// Context for the message state machine.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct MessageContext {
    pub current_time_secs: u64,
    pub total_acked: u64,
    pub total_nacked: u64,
    pub total_dead_lettered: u64,
}

state_machine! {
    pub enum RelayMessage {
        #[initial]
        Pending,
        InFlight {
            leader_id: NodeId,
            leader_term: u64,
            started_at_secs: u64,
        },
        InDoubt {
            previous_leader: NodeId,
            previous_term: u64,
            doubt_started_at_secs: u64,
        },
        Acked,
        DeadLettered { reason: DeadLetterReason },
    }

    type Context = MessageContext;

    impl RelayMessage {
        /// Pending -> InFlight: A leader claims this message for delivery.
        fn mark_in_flight(self: Pending, leader_id: NodeId, leader_term: u64) -> InFlight {
            RelayMessageStateData::InFlight {
                leader_id,
                leader_term,
                started_at_secs: ctx.current_time_secs,
            }
        }

        /// InFlight -> InDoubt: A new leader sees an in-flight message from old leader.
        /// Fails if the leader/term hasn't actually changed.
        fn mark_in_doubt(self: InFlight, new_leader: NodeId, new_term: u64) -> Result<InDoubt, MessageError> {
            // Can only go to InDoubt if leadership actually changed
            if leader_id == new_leader && leader_term == new_term {
                Err(MessageError::SameLeader)
            } else {
                Ok(RelayMessageStateData::InDoubt {
                    previous_leader: leader_id,
                    previous_term: leader_term,
                    doubt_started_at_secs: ctx.current_time_secs,
                })
            }
        }

        /// InFlight | InDoubt -> Acked: Original leader confirms delivery.
        /// Validates that the ack comes from the correct leader/term.
        fn forwarded_ack(self: (InFlight, InDoubt), original_leader: NodeId, original_term: u64) -> Result<Acked, MessageError> {
            // Extract expected leader/term based on current state
            let (expected_leader, expected_term) = match state {
                RelayMessageStateData::InFlight { leader_id, leader_term, .. } => (*leader_id, *leader_term),
                RelayMessageStateData::InDoubt { previous_leader, previous_term, .. } => (*previous_leader, *previous_term),
                _ => unreachable!(),
            };

            if original_leader != expected_leader || original_term != expected_term {
                Err(MessageError::LeaderMismatch {
                    expected_leader,
                    expected_term,
                    got_leader: original_leader,
                    got_term: original_term,
                })
            } else {
                ctx.total_acked += 1;
                Ok(RelayMessageStateData::Acked)
            }
        }

        /// InFlight | InDoubt -> Pending: Original leader reports failure, retry.
        fn forwarded_nack(self: (InFlight, InDoubt), original_leader: NodeId, original_term: u64) -> Result<Pending, MessageError> {
            let (expected_leader, expected_term) = match state {
                RelayMessageStateData::InFlight { leader_id, leader_term, .. } => (*leader_id, *leader_term),
                RelayMessageStateData::InDoubt { previous_leader, previous_term, .. } => (*previous_leader, *previous_term),
                _ => unreachable!(),
            };

            if original_leader != expected_leader || original_term != expected_term {
                Err(MessageError::LeaderMismatch {
                    expected_leader,
                    expected_term,
                    got_leader: original_leader,
                    got_term: original_term,
                })
            } else {
                ctx.total_nacked += 1;
                Ok(RelayMessageStateData::Pending)
            }
        }

        /// InDoubt -> DeadLettered: Timeout waiting for resolution.
        fn timeout_in_doubt(self: InDoubt) -> DeadLettered {
            ctx.total_dead_lettered += 1;
            RelayMessageStateData::DeadLettered {
                reason: DeadLetterReason::AmbiguousFailure {
                    reason: "leader_partition_timeout".into(),
                },
            }
        }
    }
}

#[cfg(all(test, not(feature = "async")))]
mod tests {
    use super::*;

    #[test]
    fn test_happy_path_ack() {
        let msg = RelayMessage::new();

        // Pending -> InFlight
        msg.mark_in_flight(1, 100).unwrap();

        // InFlight -> Acked
        msg.forwarded_ack(1, 100).unwrap();
    }

    #[test]
    fn test_in_doubt_then_ack() {
        let msg = RelayMessage::new();

        // Pending -> InFlight (leader 1, term 100)
        msg.mark_in_flight(1, 100).unwrap();

        // InFlight -> InDoubt (new leader 2, term 101 takes over)
        msg.mark_in_doubt(2, 101).unwrap();

        // InDoubt -> Acked (original leader 1 confirms)
        msg.forwarded_ack(1, 100).unwrap();
    }

    #[test]
    fn test_nack_and_retry() {
        let msg = RelayMessage::new();

        // Pending -> InFlight
        msg.mark_in_flight(1, 100).unwrap();

        // InFlight -> Pending (nack, will retry)
        msg.forwarded_nack(1, 100).unwrap();

        // Pending -> InFlight (retry with new leader)
        msg.mark_in_flight(2, 101).unwrap();

        // InFlight -> Acked
        msg.forwarded_ack(2, 101).unwrap();
    }

    #[test]
    fn test_in_doubt_timeout() {
        let msg = RelayMessage::new();

        // Pending -> InFlight
        msg.mark_in_flight(1, 100).unwrap();

        // InFlight -> InDoubt
        msg.mark_in_doubt(2, 101).unwrap();

        // InDoubt -> DeadLettered (timeout)
        msg.timeout_in_doubt().unwrap();
    }

    #[test]
    fn test_wrong_leader_ack_rejected() {
        let msg = RelayMessage::new();

        // Pending -> InFlight (leader 1, term 100)
        msg.mark_in_flight(1, 100).unwrap();

        // Try to ack with wrong leader
        let err = msg.forwarded_ack(2, 100).unwrap_err();
        assert_eq!(err, ssm::TransitionError::User(MessageError::LeaderMismatch {
            expected_leader: 1,
            expected_term: 100,
            got_leader: 2,
            got_term: 100,
        }));

        // Can still ack with correct credentials
        msg.forwarded_ack(1, 100).unwrap();
    }

    #[test]
    fn test_same_leader_cannot_mark_in_doubt() {
        let msg = RelayMessage::new();

        // Pending -> InFlight
        msg.mark_in_flight(1, 100).unwrap();

        // Try to mark in-doubt with same leader - should fail
        let err = msg.mark_in_doubt(1, 100).unwrap_err();
        assert_eq!(err, ssm::TransitionError::User(MessageError::SameLeader));

        // Can still ack normally
        msg.forwarded_ack(1, 100).unwrap();
    }

    #[test]
    fn test_concurrent_access() {
        let msg1 = RelayMessage::new();
        let msg2 = msg1.clone();

        // Both handles can be used
        msg1.mark_in_flight(1, 100).unwrap();

        // msg2 sees the same state
        let state = msg2.get_state();
        assert!(matches!(state, RelayMessageStateData::InFlight { leader_id: 1, leader_term: 100, .. }));

        // msg2 can continue the flow
        msg2.forwarded_ack(1, 100).unwrap();

        // Both see final state
        let state = msg1.get_state();
        assert!(matches!(state, RelayMessageStateData::Acked));
    }
}
