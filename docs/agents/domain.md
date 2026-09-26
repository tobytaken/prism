# Domain Docs

This repo uses a single-context layout: `CONTEXT.md` at the repo root and architectural decision records in `docs/adr/`.

## Before exploring

Read the root `CONTEXT.md` and ADRs in `docs/adr/` that touch the area you are about to work in.

If these files do not exist, proceed silently. The `/domain-modeling` skill creates them lazily when terms or decisions are resolved.

## Use the glossary's vocabulary

When naming a domain concept in an issue, proposal, hypothesis, or test, use the term defined in `CONTEXT.md`.

If a concept is missing, check whether an existing term fits; otherwise note the gap for `/domain-modeling`.

## Flag ADR conflicts

If a proposal contradicts an existing ADR, explicitly identify the ADR and explain why the decision should be reconsidered.
