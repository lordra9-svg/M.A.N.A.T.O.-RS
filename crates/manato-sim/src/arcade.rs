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
    /// Previously persisted best score.
    pub high_score: u64,
}

impl Default for ArcadeConfig {
    fn default() -> Self {
        Self {
            credits: 3,
            continue_seconds: 10,
            survival: false,
            high_score: 0,
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
    pub high_score: u64,
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
            high_score: config.high_score,
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
            // Survival keeps facing new opponents from the ladder.
            if !self.ladder.is_empty() {
                self.current = (self.current + 1) % self.ladder.len();
            }
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
        self.high_score = self.high_score.max(self.score);
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

pub const SCORE_TABLE_SIZE: usize = 5;

/// Builds an arcade ladder from roster order. `random_flags[i]` marks a
/// `randomselect` entry, which is replaced by a deterministic pick from the
/// non-random roster (seeded by `seed`). `p1` is excluded.
pub fn build_ladder(random_flags: &[bool], p1: usize, seed: u64) -> Vec<usize> {
    let candidates: Vec<usize> = (0..random_flags.len())
        .filter(|index| *index != p1 && !random_flags[*index])
        .collect();
    let mut state = seed | 1;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) as usize
    };
    let mut ladder = Vec::new();
    for (index, is_random) in random_flags.iter().enumerate() {
        if index == p1 {
            continue;
        }
        if *is_random && !candidates.is_empty() {
            ladder.push(candidates[next() % candidates.len()]);
        } else {
            ladder.push(index);
        }
    }
    if ladder.is_empty() {
        ladder.push(p1);
    }
    ladder
}

/// Reads the persisted score table (highest first).
pub fn load_score_table(path: &std::path::Path) -> Vec<u64> {
    let mut scores: Vec<u64> = std::fs::read_to_string(path)
        .ok()
        .map(|text| {
            text.lines()
                .filter_map(|line| line.trim().parse::<u64>().ok())
                .collect()
        })
        .unwrap_or_default();
    scores.sort_unstable_by(|a, b| b.cmp(a));
    scores.truncate(SCORE_TABLE_SIZE);
    scores
}

/// Writes the score table (already ordered).
pub fn save_score_table(path: &std::path::Path, scores: &[u64]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text: String = scores
        .iter()
        .take(SCORE_TABLE_SIZE)
        .map(|score| format!("{score}\n"))
        .collect();
    std::fs::write(path, text)
}

/// Inserts `score` into the descending table, keeping the top entries.
pub fn insert_score(scores: &mut Vec<u64>, score: u64) {
    if score == 0 {
        return;
    }
    scores.push(score);
    scores.sort_unstable_by(|a, b| b.cmp(a));
    scores.truncate(SCORE_TABLE_SIZE);
}

/// Reads a persisted high score; missing or malformed files read as 0.
pub fn load_high_score(path: &std::path::Path) -> u64 {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

/// Persists a high score, creating parent directories when needed.
pub fn save_high_score(path: &std::path::Path, score: u64) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{score}\n"))
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
                high_score: 0,
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
            vec![10, 20],
            ArcadeConfig {
                credits: 0,
                continue_seconds: 1,
                survival: true,
                high_score: 0,
            },
        );
        session.win(0, false, 100);
        assert_eq!(session.current, 1, "survival rotates opponents");
        session.win(0, false, 50);
        assert_eq!(session.current, 0);
        assert_eq!(session.streak, 2);
        assert_eq!(session.score, (300 + 100 * 5) + (300 + 100 + 50 * 5));
        session.lose();
        assert_eq!(session.phase, ArcadePhase::GameOver);
    }

    #[test]
    fn ladder_resolves_random_slots_deterministically() {
        let flags = [false, false, true, false];
        let ladder = build_ladder(&flags, 0, 42);
        assert_eq!(ladder.len(), 3, "player 0 excluded, one slot per opponent");
        assert!(!ladder.contains(&0), "player 0 excluded");
        assert!(!ladder.contains(&2), "random marker replaced");
        assert!(ladder.contains(&1) && ladder.contains(&3));
        // Same seed produces the same ladder.
        assert_eq!(ladder, build_ladder(&flags, 0, 42));
        // A single-entry roster still yields one opponent.
        assert_eq!(build_ladder(&[false], 0, 1), vec![0]);
    }

    #[test]
    fn score_table_sorts_trims_and_persists() {
        let dir = std::env::temp_dir().join(format!("manato-table-{}", std::process::id()));
        let path = dir.join("scores.dat");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(load_score_table(&path).is_empty());
        let mut table = vec![100, 500];
        insert_score(&mut table, 300);
        insert_score(&mut table, 900);
        insert_score(&mut table, 0);
        assert_eq!(table, vec![900, 500, 300, 100]);
        for score in [50, 25, 75] {
            insert_score(&mut table, score);
        }
        assert_eq!(table, vec![900, 500, 300, 100, 75]);
        save_score_table(&path, &table).expect("save table");
        assert_eq!(load_score_table(&path), table);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn high_score_persists() {
        let dir = std::env::temp_dir().join(format!("manato-score-{}", std::process::id()));
        let path = dir.join("score.dat");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load_high_score(&path), 0);
        save_high_score(&path, 1234).expect("save");
        assert_eq!(load_high_score(&path), 1234);
        save_high_score(&path, 9999).expect("overwrite");
        assert_eq!(load_high_score(&path), 9999);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
