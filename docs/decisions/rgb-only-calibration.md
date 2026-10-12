# Store calibration references as RGB values only

Status: accepted; replaces the original provenance-bearing reference design. The original photo-fingerprint clauses below are superseded by [removing RAW content hashing](no-raw-content-hashing.md); RGB-only calibration and development compatibility remain unchanged.

## Context

Remembered source/rectangle relationships caused unwanted navigation on re-picking and coupled measured calibration to photo management. The user explicitly defined Dmin/Dmax as one RGB value each, independent of where they were measured.

## Decision

Persist only Dmin/Dmax RGB. Keep picker source, rectangle and working-photo return context transient. Re-picking starts on the current photo with an empty rectangle. Keep development/basis/floor/model compatibility at roll level and source identity/fingerprints with photos for relinking.

## Consequences

References survive missing, changed or removed photos; such events never silently resample them. Full-resolution means, calibrated-domain checks and invalid/cancelled-pick atomicity remain required. Schema 3 rejects older provenance-bearing documents, rather than preserving that obsolete requirement through a compatibility shim. This decision does not remove photo fingerprint/development compatibility checks.

## Evidence

- [Processing contract](../contracts/processing-model.md#inversion-and-validation), [project foundation](../contracts/project-state.md).
- [Approved reference cleanup and user requirement](../research/history/feedback-plan.md#2-simplify-references-and-build-consistent-editing-controls).
- `src/editor.rs`, `src/project.rs`, `src/editor_tests.rs`, `tests/project.rs`.
