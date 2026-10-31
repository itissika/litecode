//! Characterization: the session layer must not grow a second turn orchestrator
//! or absorb the subagent job registry. Tools call the same primitives humans
//! use (`reserve_turn` → `spawn_turn` → `start_turn`).
