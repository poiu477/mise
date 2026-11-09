//! The demo recipes (`fixtures/demo-bundle`, written for mise) loaded into a test's database through `mise-ingest`,
//! in the order `web/e2e/serve.mjs` runs the same stages with the `mise` command.

use std::path::{Path, PathBuf};

use super::{MediaDir, TestApp};

/// The demo bundle in the repository.
pub fn bundle_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/demo-bundle")
}

impl TestApp {
    /// The app on a database holding the demo recipes, with an empty media folder of its own; None when there is no
    /// database (outside CI).
    pub async fn with_demo() -> Option<TestApp> {
        let app = TestApp::start(None, Some(MediaDir::create())).await?;
        app.load_demo().await;
        Some(app)
    }

    /// Import the demo bundle and run every stage after it: match foods, apply the curation file, group dishes,
    /// derive options, parse steps. Panics, naming the stage, if one fails.
    pub async fn load_demo(&self) {
        let dir = bundle_dir();
        let pool = &self.pool;
        mise_ingest::bundle::import_bundle(pool, &dir)
            .await
            .expect("import the demo bundle");
        let foods = dir.join("foods.ndjson");
        if foods.exists() {
            mise_ingest::foods::import_food_knowledge(pool, &foods)
                .await
                .expect("import the demo food knowledge");
        }
        mise_ingest::foods::match_foods(pool)
            .await
            .expect("match foods");
        mise_ingest::curate::apply_file(pool, &dir.join("curation.json"))
            .await
            .expect("apply the demo curation");
        mise_ingest::dishes::group_dishes(pool)
            .await
            .expect("group dishes");
        mise_ingest::options::derive_options(pool)
            .await
            .expect("derive options");
        mise_ingest::steps::parse_steps(pool)
            .await
            .expect("parse steps");
    }
}
