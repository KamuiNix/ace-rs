use anyhow::{anyhow, bail, Result};
use directories::BaseDirs;
use indicatif::{ProgressBar, ProgressStyle};
use rusqlite::{params, Connection, Transaction};
use serde_derive::Deserialize;
use std::convert::TryInto;
use std::{fs, path::Path};

use crate::deinflect;

#[derive(Debug)]
pub struct DictConn {
    pub conn: Connection,
    pub new: bool,
}

#[derive(Debug, Deserialize)]
struct YomichanDict {
    title: String,
}

#[derive(Debug)]
pub struct DbDictionary {
    id: i64,
    pub title: String,
    pub priority: i64,
    pub fallback: bool,
    pub enabled: bool,
}

#[derive(Debug)]
pub struct DbDictEntry {
    pub id: i64,
    pub kanji: String,
    pub reading: String,
    pub meaning: String,
    pub dict_id: i64,
}

#[derive(Debug, Deserialize)]
struct YomichanEntryV1 {
    kanji: String,
    reading: String,
    definition_tags: String,
    rule_identifiers: String,
    popularity: i32,
    meanings: Vec<String>,
    sequence: usize,
    term_tags: String,
}

pub struct DictDb {
    conn: DictConn,
}

impl DictDb {
    pub fn new() -> Result<Self> {
        let conn = DictConn::new()?;
        if conn.new {
            let res = conn.setup_schema();
            if res.is_err() {
                bail!("Unable to setup database schema")
            }
        }
        Ok(DictDb { conn })
    }

    pub fn load_yomichan_dict(&mut self, path: &Path, title: String) -> Result<()> {
        if Self::validate_yomichan(path) {
            // setup transaction for faster writes
            let tx = self.conn.get_transaction()?;

            if Self::get_dict_id(&title, &tx).is_ok() {
                return Ok(());
            }

            let paths = fs::read_dir(path)?
                .filter(|path| {
                    let filename = path.as_ref().unwrap().file_name();
                    let filename = filename.to_str().unwrap();
                    filename.starts_with("term_bank_") && filename.ends_with(".json")
                })
                .collect::<Result<Vec<_>, std::io::Error>>()?;

            let total = paths.len();

            let dict_id = Self::insert_dict(&title, &tx)?;
            for (index, term_bank) in paths.iter().enumerate() {
                let text = std::fs::read_to_string(&term_bank.path())?;
                let data: Vec<YomichanEntryV1> = serde_json::from_str(&text)?;
                let msg = format!("{}/{}", index + 1, total);
                let bar = ProgressBar::new(data.len().try_into().unwrap()).with_message(msg);
                bar.set_style(
                    ProgressStyle::default_bar()
                        .template(
                            "{msg} {spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos:>7}/{len:7}",
                        )
                        .progress_chars("#>-"),
                );

                for entry in data {
                    Self::insert_entry(entry, dict_id, &tx)?;
                    bar.inc(1);
                }

                bar.finish_and_clear();
            }

            if tx.commit().is_err() {
                bail!("Unable to commit transaction");
            }

            println!("Finished importing dictionary.");
        }
        Ok(())
    }

    pub fn update_dict(
        &self,
        title: &str,
        new_priority: i64,
        new_fallback: i8,
        enabled: i8,
    ) -> rusqlite::Result<usize> {
        self.conn.conn.execute(
            "UPDATE dicts SET priority = ?2, fallback = ?3, enabled = ?4 WHERE title = ?1",
            params![title, new_priority, new_fallback, enabled],
        )
    }

    pub fn rename_dict(&self, old: &str, new: &str) -> rusqlite::Result<usize> {
        self.conn.conn.execute(
            "UPDATE dicts SET title = ?2 WHERE title = ?1",
            params![old, new],
        )
    }

    pub fn get_all_dicts(&self) -> rusqlite::Result<Vec<DbDictionary>> {
        let mut stmt = self.conn.conn.prepare("SELECT * FROM dicts")?;
        let mut rows = stmt.query([])?;

        let mut dicts = Vec::new();
        while let Some(row) = rows.next()? {
            dicts.push(DbDictionary {
                id: row.get(0)?,
                title: row.get(1)?,
                priority: row.get(2)?,
                fallback: row.get(3)?,
                enabled: row.get(4)?,
            })
        }

        Ok(dicts)
    }

    pub fn validate_yomichan(path: &Path) -> bool {
        let is_dir = path.is_dir();
        let has_index = path.join("index.json").exists();
        let has_termbanks = path.join("term_bank_1.json").exists();
        is_dir && has_index && has_termbanks
    }

    fn get_dict_id(title: &str, tx: &Transaction) -> rusqlite::Result<i64> {
        let dict_id = tx.query_row::<i64, _, _>(
            "SELECT id FROM dicts WHERE title = ?1 LIMIT 1",
            params![title],
            |r| r.get(0),
        )?;

        Ok(dict_id)
    }

    fn insert_dict(title: &str, tx: &Transaction) -> rusqlite::Result<i64> {
        tx.query_row::<i64, _, _>(
            "INSERT INTO dicts (title) VALUES (?1) RETURNING id",
            params![title],
            |r| r.get(0),
        )
    }

    fn insert_entry(
        entry: YomichanEntryV1,
        dict_id: i64,
        tx: &Transaction,
    ) -> rusqlite::Result<()> {
        for meaning in entry.meanings {
            tx.execute(
                "INSERT INTO entries (kanji, reading, meaning, dict_id) VALUES (?1, ?2, ?3, ?4)",
                params![entry.kanji, entry.reading, meaning, dict_id],
            )?;
        }

        Ok(())
    }

    fn _lookup_word(&self, word: &str, fallback: bool) -> rusqlite::Result<Vec<DbDictEntry>> {
        let mut stmt = self
            .conn
            .conn
            .prepare("SELECT * FROM entries INNER JOIN dicts ON entries.dict_id = dicts.id WHERE enabled = 1 AND fallback = :fallback AND kanji = :word ORDER BY priority DESC")?;
        let rows = stmt.query_map(
            &[
                (":word", word),
                (":fallback", &(fallback as i32).to_string()),
            ],
            |row| {
                Ok(DbDictEntry {
                    id: row.get(0)?,
                    kanji: row.get(1)?,
                    reading: row.get(2)?,
                    meaning: row.get(3)?,
                    dict_id: row.get(4)?,
                })
            },
        )?;
        let mut entries: Vec<DbDictEntry> = vec![];
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }

    pub fn lookup_word(&self, word: &str) -> rusqlite::Result<Vec<DbDictEntry>> {
        let mut entries = self._lookup_word(word, false)?;
        // fallback
        if entries.is_empty() {
            entries = self._lookup_word(word, true)?;
        }

        Ok(entries)
    }
}

impl DictConn {
    pub fn new() -> Result<Self> {
        let basedirs = BaseDirs::new();
        if let Some(basedirs) = basedirs {
            let path = basedirs.data_dir().join("ace");
            fs::create_dir_all(&path)?;
            let db_path = path.join("dict.db");
            let new = !db_path.exists();
            let conn = Connection::open(db_path)?;

            Ok(DictConn { conn, new })
        } else {
            Err(anyhow!("Could not find data directory"))
        }
    }

    pub fn get_transaction(&mut self) -> rusqlite::Result<Transaction> {
        self.conn.transaction()
    }

    pub fn setup_schema(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS dicts (
                  id              INTEGER PRIMARY KEY AUTOINCREMENT,
                  title           TEXT NOT NULL,
                  priority        INTEGER DEFAULT 0,
                  fallback        INTEGER DEFAULT 0,
                  enabled         INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS entries (
                  id              INTEGER PRIMARY KEY AUTOINCREMENT,
                  kanji           TEXT NOT NULL,
                  reading         TEXT,
                  meaning         TEXT NOT NULL,
                  dict_id         INTEGER NOT NULL,
                  FOREIGN KEY(dict_id) REFERENCES dicts(id)
            );

            CREATE INDEX IF NOT EXISTS kanji_idx ON entries(kanji); 
            ",
        )
    }
}

pub fn lookup(dict_db: &DictDb, word: String) -> Result<Vec<DbDictEntry>> {
    let mut results: Vec<DbDictEntry> = vec![];
    let deinflect_json = include_str!("../data/deinflect.json");
    let deinflector = deinflect::Deinflector::new(deinflect_json);
    let deinflected_forms = deinflector.deinflect(word);

    for form in deinflected_forms {
        let lookup_res = dict_db.lookup_word(&form.term)?;
        results.extend(lookup_res);
    }

    Ok(results)
}
