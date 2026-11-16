//! Arcade ladder, continue, and survival flow (`docs/spec/arcade.md`).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArcadePhase {
    /// Playing the current ladder match.
    Fight,
    /// Lost a match with credits left; countdown running.
    Continue,
    /// Beat the final ladder opponent.
    Complete,
    /// Out of credits or the countdown expired.
    GameOver,
}

#[derive(Debug, Clone, Copy)]
pub struct ArcadeConfig {
    pub credits: u32,
    /// Continue countdown in seconds (60 ticks per second).
    pub continue_seconds: i32,
    pub survival: bool,
}

impl Default for ArcadeConfig {
    fn default() -> Self {
        Self {
            credits: 3,
            continue_seconds: 10,
            survival: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ArcadeSession {
    pub ladder: Vec<usize>,
    pub current: usize,
    pub phase: ArcadePhase,
    pub credits: u32,
    pub continue_timer: i32,
    pub score: u64,
    pub streak: u32,
    pub wins: u32,
    pub losses: u32,
    pub continues: u32,
    pub survival: bool,
    continue_seconds: i32,
}

impl ArcadeSession {
    pub fn new(ladder: Vec<usize>, config: ArcadeConfig) -> Self {
        Self {
            current: 0,
            phase: if ladder.is_empty() {
                ArcadePhase::Complete
            } else {
                ArcadePhase::Fight
            },
            ladder,
            credits: config.credits,
            continue_timer: 0,
            score: 0,
            streak: 0,
            wins: 0,
            losses: 0,
            continues: 0,
            survival: config.survival,
            continue_seconds: config.continue_seconds.max(1),
        }
    }

    pub fn current_opponent(&self) -> Option<usize> {
        self.ladder.get(self.current).copied()
    }

    /// Records a won match. `time_left` is the round timer, `perfect` whether
    /// the player took no damage, and `life_left` the remaining life (used by
    /// survival scoring).
    pub fn win(&mut self, time_left: i32, perfect: bool, life_left: i32) {
        if !matches!(self.phase, ArcadePhase::Fight) {
            return;
        }
        self.wins += 1;
        self.streak += 1;
        if self.survival {
            self.score +=
                300 + u64::from(self.streak.saturating_sub(1)) * 100 + life_left.max(0) as u64 * 5;
            // Survival stays on the same opponent; the host may rotate it.
        } else {
            self.score += 1000 + time_left.max(0) as u64 * 10;
            if perfect {
                self.score += 1000;
            }
            self.current += 1;
            if self.current >= self.ladder.len() {
                self.phase = ArcadePhase::Complete;
            }
        }
    }

    /// Records a lost match. Remaining credits start a continue countdown;
    /// otherwise the session ends.
    pub fn lose(&mut self) {
        if !matches!(self.phase, ArcadePhase::Fight) {
            return;
        }
        self.losses += 1;
        self.streak = 0;
        if self.credits > 0 {
            self.phase = ArcadePhase::Continue;
            self.continue_timer = self.continue_seconds * 60;
        } else {
            self.phase = ArcadePhase::GameOver;
        }
    }

    /// Advances the continue countdown by one tick.
    pub fn tick(&mut self) {
        if self.phase != ArcadePhase::Continue {
            return;
        }
        self.continue_timer = (self.continue_timer - 1).max(0);
        if self.continue_timer == 0 {
            self.phase = ArcadePhase::GameOver;
        }
    }

    /// Spends a credit to resume the current match.
    pub fn use_credit(&mut self) -> bool {
        if self.phase != ArcadePhase::Continue || self.credits == 0 {
            return false;
        }
        self.credits -= 1;
        self.continues += 1;
        self.continue_timer = 0;
        self.phase = ArcadePhase::Fight;
        true
    }

    /// Seconds remaining on the continue countdown.
    pub fn continue_seconds_left(&self) -> i32 {
        (self.continue_timer + 59) / 60
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ladder_advances_and_completes() {
        let mut session = ArcadeSession::new(vec![10, 20], ArcadeConfig::default());
        assert_eq!(session.current_opponent(), Some(10));
        session.win(50, false, 0);
        assert_eq!(session.current_opponent(), Some(20));
        assert_eq!(session.phase, ArcadePhase::Fight);
        session.win(0, true, 0);
        assert_eq!(session.phase, ArcadePhase::Complete);
        assert_eq!(session.wins, 2);
        assert_eq!(session.score, (1000 + 50 * 10) + (1000 + 1000));
    }

    #[test]
    fn continue_consumes_credits_and_expires() {
        let mut session = ArcadeSession::new(
            vec![10],
            ArcadeConfig {
                credits: 2,
                continue_seconds: 3,
                survival: false,
            },
        );
        session.lose();
        assert_eq!(session.phase, ArcadePhase::Continue);
        assert_eq!(session.continue_seconds_left(), 3);
        assert!(session.use_credit());
        assert_eq!(session.credits, 1);
        assert_eq!(session.continues, 1);
        assert_eq!(session.phase, ArcadePhase::Fight);

        session.lose();
        for _ in 0..180 {
            session.tick();
        }
        assert_eq!(session.phase, ArcadePhase::GameOver);
        assert!(!session.use_credit(), "no resume after timeout");
    }

    #[test]
    fn survival_accumulates_score_and_streak() {
        let mut session = ArcadeSession::new(
            vec![10],
            ArcadeConfig {
                credits: 0,
                continue_seconds: 1,
                survival: true,
            },
        );
        session.win(0, false, 100);
        session.win(0, false, 50);
        assert_eq!(session.current, 0, "survival stays on the opponent");
        assert_eq!(session.streak, 2);
        assert_eq!(session.score, (300 + 100 * 5) + (300 + 100 + 50 * 5));
        session.lose();
        assert_eq!(session.phase, ArcadePhase::GameOver);
    }
}
