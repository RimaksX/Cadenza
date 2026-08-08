# 4. No neural networks, no model downloads

Status: accepted (M0)

## Context

Smart shuffle and smart radio need to judge musical similarity and mood. The obvious
modern answer is an embedding model plus a vector index. Cadenza is offline by design,
ships no model weights, and must keep background CPU under roughly 20%.

## Decision

Similarity and mood are computed from lightweight DSP features only: BPM, key and mode,
energy, loudness, spectral centroid and rolloff, danceability and valence heuristics,
tempo stability, dynamic range. Ranking uses explicit weighted formulas
(`PROJECT_MASTER.json` sections 9 and 10), user history, and skip/like/dislike feedback.

Explicitly excluded: neural networks, ONNX, embedding models, ChromaDB, FAISS, and any
external ML service.

## Consequences

- No downloads, no network access, no model licensing, no multi-hundred-megabyte
  installer.
- Feature extraction runs once per file, is cached in `track_features`, and is versioned
  by `extractor_version` so re-analysis only happens when the algorithm changes.
- Recommendation quality is bounded by the quality of the heuristics. Tune the weights;
  do not reach for a model.
- The formulas are testable and explainable — a radio pick can state why it was chosen
  (`radio_session_items.reason_json`).

Reversing this decision requires amending `PROJECT_MASTER.json` first.
