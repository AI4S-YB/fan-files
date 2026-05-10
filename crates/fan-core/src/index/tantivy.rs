use std::path::Path;
use tantivy::schema::{Schema, SchemaBuilder, STORED, TEXT};
use tantivy::schema::Value;
use tantivy::{Index, IndexWriter, ReloadPolicy, TantivyDocument};
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::directory::MmapDirectory;
use std::sync::Mutex;

pub struct TantivyIndex {
    index: Index,
    writer: Mutex<IndexWriter>,
    schema: Schema,
}

impl TantivyIndex {
    pub fn open(data_dir: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let index_dir = data_dir.join("tantivy");
        std::fs::create_dir_all(&index_dir).ok();

        let mut schema_builder = SchemaBuilder::new();
        schema_builder.add_i64_field("file_id", STORED);
        schema_builder.add_text_field("path", TEXT | STORED);
        schema_builder.add_text_field("metadata", TEXT);
        schema_builder.add_text_field("tags", TEXT);
        let schema = schema_builder.build();

        let dir = MmapDirectory::open(&index_dir)?;
        let index = Index::open_or_create(dir, schema.clone())?;

        let writer = index.writer(50_000_000)?;

        Ok(Self {
            index,
            writer: Mutex::new(writer),
            schema,
        })
    }

    pub fn index_file(
        &self,
        id: i64,
        path: &Path,
        metadata_text: &str,
        tags: &[&str],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let file_id_field = self.schema.get_field("file_id").unwrap();
        let path_field = self.schema.get_field("path").unwrap();
        let metadata_field = self.schema.get_field("metadata").unwrap();
        let tags_field = self.schema.get_field("tags").unwrap();

        let mut doc = TantivyDocument::new();
        doc.add_i64(file_id_field, id);
        doc.add_text(path_field, &path.to_string_lossy());
        doc.add_text(metadata_field, metadata_text);
        doc.add_text(tags_field, tags.join(" "));

        // Delete any existing doc for this file_id, then add the new one
        let mut writer = self.writer.lock().unwrap();
        let id_term = tantivy::Term::from_field_i64(file_id_field, id);
        writer.delete_term(id_term);
        writer.add_document(doc)?;
        writer.commit()?;

        Ok(())
    }

    pub fn search(
        &self,
        query_str: &str,
        limit: usize,
    ) -> Result<Vec<(i64, f32)>, Box<dyn std::error::Error>> {
        let reader = self
            .index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;
        let searcher = reader.searcher();

        let metadata_field = self.schema.get_field("metadata").unwrap();
        let path_field = self.schema.get_field("path").unwrap();
        let tags_field = self.schema.get_field("tags").unwrap();
        let file_id_field = self.schema.get_field("file_id").unwrap();

        let query_parser = QueryParser::for_index(&self.index, vec![
            metadata_field,
            path_field,
            tags_field,
        ]);

        let query = query_parser.parse_query(query_str)?;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(limit))?;

        let mut results = Vec::new();
        for (score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher.doc::<TantivyDocument>(doc_address)?;
            let file_id = doc
                .get_first(file_id_field)
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            results.push((file_id, score));
        }

        Ok(results)
    }

    pub fn delete(&self, file_id: i64) -> Result<(), Box<dyn std::error::Error>> {
        let file_id_field = self.schema.get_field("file_id").unwrap();
        let mut writer = self.writer.lock().unwrap();
        let id_term = tantivy::Term::from_field_i64(file_id_field, file_id);
        writer.delete_term(id_term);
        writer.commit()?;
        Ok(())
    }
}
