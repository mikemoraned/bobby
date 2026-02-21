use std::path::Path;

use rusqlite::{params, Connection};

use crate::scoring::CandidateScore;

pub struct CandidateDb {
    conn: Connection,
}

impl CandidateDb {
    pub fn new(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS candidates (
                id TEXT PRIMARY KEY,
                discovered_at TEXT NOT NULL,
                original_at_us INTEGER NOT NULL,
                original_path TEXT NOT NULL,
                annotated_path TEXT NOT NULL,
                score_overall REAL NOT NULL,
                score_face_position REAL NOT NULL,
                score_overlap REAL NOT NULL,
                score_avg_certainty REAL NOT NULL
            )",
        )?;
        Ok(Self { conn })
    }

    pub fn insert(
        &self,
        id: &str,
        discovered_at: &str,
        original_at_us: u64,
        original_path: &str,
        annotated_path: &str,
        score: &CandidateScore,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO candidates
             (id, discovered_at, original_at_us, original_path, annotated_path,
              score_overall, score_face_position, score_overlap, score_avg_certainty)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                discovered_at,
                original_at_us,
                original_path,
                annotated_path,
                score.overall,
                score.face_position,
                score.overlap,
                score.avg_certainty,
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_created_on_new_db() {
        let db = CandidateDb::new(Path::new(":memory:")).unwrap();
        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM candidates", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn insert_and_query_candidate() {
        let db = CandidateDb::new(Path::new(":memory:")).unwrap();
        let score = CandidateScore {
            face_position: 1.0,
            overlap: 0.95,
            avg_certainty: 0.85,
            overall: 0.8075,
        };
        db.insert(
            "test_abc_0",
            "2024-01-15T10:30:00Z",
            1700000000000000,
            "candidates/test_abc_0.png",
            "candidates/test_abc_0_annotated.png",
            &score,
        )
        .unwrap();

        let (id, overall, original_at): (String, f64, u64) = db
            .conn
            .query_row(
                "SELECT id, score_overall, original_at_us FROM candidates WHERE id = ?1",
                ["test_abc_0"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(id, "test_abc_0");
        assert!((overall - 0.8075).abs() < 1e-4);
        assert_eq!(original_at, 1700000000000000);
    }
}
