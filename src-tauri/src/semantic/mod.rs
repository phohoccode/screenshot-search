pub mod engine;
pub mod input;
pub mod manager;

pub use engine::{
    FastembedModelEngine, MockEmbeddingEngine, TextEmbeddingEngine, DEFAULT_EMBEDDING_DIM,
    DEFAULT_MODEL_ID, DEFAULT_MODEL_VERSION,
};
pub use input::format_semantic_document;
pub use manager::{SemanticModelInfo, SemanticModelManager, SemanticModelStatus};

#[cfg(test)]
pub mod semantic_tests;
