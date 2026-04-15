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
