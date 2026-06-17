use lancedb::index::IndexStatistics;
use lancedb::table::TableStatistics;

pub struct TableHealth {
    pub name: String,
    pub stats: TableStatistics,
    pub index_health: Vec<IndexHealth>,
}

pub struct IndexHealth {
    pub name: String,
    pub stats: Option<IndexStatistics>,
}

impl TableHealth {
    pub const fn needs_compaction(&self) -> bool {
        self.stats.fragment_stats.num_small_fragments > 10
    }

    pub fn needs_index_rebuild(&self) -> bool {
        self.index_health.iter().any(|idx| {
            idx.stats.as_ref().is_some_and(|s| {
                s.num_unindexed_rows > 0
                    && s.num_unindexed_rows * 10 > s.num_indexed_rows + s.num_unindexed_rows
            })
        })
    }

    pub fn print_report(&self) {
        println!("=== {} ===", self.name);
        println!(
            "  rows: {}  fragments: {} (small: {})",
            self.stats.num_rows,
            self.stats.fragment_stats.num_fragments,
            self.stats.fragment_stats.num_small_fragments,
        );
        let lengths = &self.stats.fragment_stats.lengths;
        println!(
            "  fragment lengths: min={} max={} mean={} p50={} p99={}",
            lengths.min, lengths.max, lengths.mean, lengths.p50, lengths.p99,
        );

        for idx in &self.index_health {
            print!("  index '{}': ", idx.name);
            match &idx.stats {
                Some(s) => {
                    let total = s.num_indexed_rows + s.num_unindexed_rows;
                    let pct_unindexed = if total > 0 {
                        s.num_unindexed_rows as f64 / total as f64 * 100.0
                    } else {
                        0.0
                    };
                    println!(
                        "indexed={} unindexed={} ({:.1}%) type={:?}",
                        s.num_indexed_rows, s.num_unindexed_rows, pct_unindexed, s.index_type,
                    );
                }
                None => println!("stats unavailable"),
            }
        }

        let mut recommendations = Vec::new();
        if self.needs_compaction() {
            recommendations.push(format!(
                "compact: {} small fragments (>10 threshold)",
                self.stats.fragment_stats.num_small_fragments,
            ));
        }
        if self.needs_index_rebuild() {
            recommendations.push("rebuild indices: >10% rows unindexed".to_string());
        }
        if recommendations.is_empty() {
            println!("  status: healthy");
        } else {
            for r in &recommendations {
                println!("  RECOMMEND: {r}");
            }
        }
        println!();
    }
}

pub struct StoreHealth {
    pub tables: Vec<TableHealth>,
}

impl StoreHealth {
    pub fn needs_action(&self) -> bool {
        self.tables
            .iter()
            .any(|t| t.needs_compaction() || t.needs_index_rebuild())
    }

    pub fn print_report(&self) {
        for table in &self.tables {
            table.print_report();
        }
        if self.needs_action() {
            println!("Overall: compaction recommended");
        } else {
            println!("Overall: healthy, no action needed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lancedb::index::IndexType;
    use lancedb::table::{FragmentStatistics, FragmentSummaryStats, TableStatistics};

    fn zero_lengths() -> FragmentSummaryStats {
        FragmentSummaryStats {
            min: 0,
            max: 0,
            mean: 0,
            p25: 0,
            p50: 0,
            p75: 0,
            p99: 0,
        }
    }

    fn stats(small_fragments: usize) -> TableStatistics {
        TableStatistics {
            total_bytes: 0,
            num_rows: 100,
            num_indices: 1,
            fragment_stats: FragmentStatistics {
                num_fragments: 20,
                num_small_fragments: small_fragments,
                lengths: zero_lengths(),
            },
        }
    }

    fn table_health(small_fragments: usize, index_health: Vec<IndexHealth>) -> TableHealth {
        TableHealth {
            name: "test".to_string(),
            stats: stats(small_fragments),
            index_health,
        }
    }

    fn index_stats(indexed: usize, unindexed: usize) -> IndexHealth {
        IndexHealth {
            name: "idx".to_string(),
            stats: Some(IndexStatistics {
                num_indexed_rows: indexed,
                num_unindexed_rows: unindexed,
                index_type: IndexType::BTree,
                distance_type: None,
                num_indices: None,
                loss: None,
            }),
        }
    }

    #[test]
    fn needs_compaction_above_threshold() {
        assert!(table_health(11, vec![]).needs_compaction());
    }

    #[test]
    fn no_compaction_at_threshold() {
        assert!(!table_health(10, vec![]).needs_compaction());
    }

    #[test]
    fn no_compaction_below_threshold() {
        assert!(!table_health(5, vec![]).needs_compaction());
    }

    #[test]
    fn needs_index_rebuild_when_over_ten_percent_unindexed() {
        // 2 unindexed out of 12 total = 16.7% > 10%
        assert!(table_health(0, vec![index_stats(10, 2)]).needs_index_rebuild());
    }

    #[test]
    fn no_index_rebuild_when_under_ten_percent() {
        // 1 unindexed out of 100 total = 1% < 10%
        assert!(!table_health(0, vec![index_stats(99, 1)]).needs_index_rebuild());
    }

    #[test]
    fn no_index_rebuild_when_fully_indexed() {
        assert!(!table_health(0, vec![index_stats(100, 0)]).needs_index_rebuild());
    }

    #[test]
    fn no_index_rebuild_when_no_indices() {
        assert!(!table_health(0, vec![]).needs_index_rebuild());
    }

    #[test]
    fn no_index_rebuild_when_stats_unavailable() {
        let idx = IndexHealth {
            name: "idx".to_string(),
            stats: None,
        };
        assert!(!table_health(0, vec![idx]).needs_index_rebuild());
    }

    #[test]
    fn print_report_does_not_panic() {
        let t = table_health(15, vec![index_stats(90, 10)]);
        t.print_report();
    }

    #[test]
    fn store_health_needs_action_when_compaction_needed() {
        let store = StoreHealth {
            tables: vec![table_health(20, vec![])],
        };
        assert!(store.needs_action());
    }

    #[test]
    fn store_health_no_action_when_healthy() {
        let store = StoreHealth {
            tables: vec![table_health(0, vec![index_stats(100, 0)])],
        };
        assert!(!store.needs_action());
    }

    #[test]
    fn store_health_print_report_does_not_panic() {
        let store = StoreHealth {
            tables: vec![table_health(0, vec![])],
        };
        store.print_report();
    }
}
