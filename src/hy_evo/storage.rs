//! Persistent Genome Storage (Task 35)
use crate::hy_evo::genome::WorkflowGenome;
use sqlx::sqlite::SqlitePool;
use sqlx::Row;

pub struct GenomeStore {
    pool: SqlitePool,
}

impl GenomeStore {
    pub async fn new(database_url: &str) -> anyhow::Result<Self> {
        let pool = SqlitePool::connect(database_url).await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS genomes (
                id TEXT PRIMARY KEY,
                data TEXT NOT NULL,
                score REAL,
                created_at INTEGER
            )",
        )
        .execute(&pool)
        .await?;

        Ok(Self { pool })
    }

    pub async fn save(&self, genome: &WorkflowGenome) -> anyhow::Result<()> {
        let data = serde_json::to_string(genome)?;
        sqlx::query(
            "INSERT OR REPLACE INTO genomes (id, data, score, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(genome.id.to_string())
        .bind(data)
        .bind(genome.score)
        .bind(chrono::Utc::now().timestamp())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn load(&self, id: &str) -> anyhow::Result<Option<WorkflowGenome>> {
        let row = sqlx::query("SELECT data FROM genomes WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        if let Some(row) = row {
            let data: String = row.get("data");
            let genome: WorkflowGenome = serde_json::from_str(&data)?;
            Ok(Some(genome))
        } else {
            Ok(None)
        }
    }

    pub async fn list_all(&self) -> anyhow::Result<Vec<WorkflowGenome>> {
        let rows = sqlx::query("SELECT data FROM genomes")
            .fetch_all(&self.pool)
            .await?;

        let mut genomes = Vec::new();
        for row in rows {
            let data: String = row.get("data");
            genomes.push(serde_json::from_str(&data)?);
        }
        Ok(genomes)
    }
}
