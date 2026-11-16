# Domain docs

## Read before exploring

Read root `GLOSSARY.md` and relevant decisions in `docs/adr/`.
If root `GLOSSARY-MAP.md` exists, follow its pointers to relevant
context glossaries and check context-scoped ADRs.

If these files are absent, proceed silently. Domain modeling creates
them lazily when terms or decisions are resolved.

## Layout

This repo uses a single context:
- `GLOSSARY.md`: shared domain vocabulary.
- `docs/adr/`: architecture decisions.

## Vocabulary and decisions

Use glossary terms in issues, proposals, hypotheses, and tests.
Reconsider invented terminology; note real vocabulary gaps for domain modeling.

Explicitly flag proposals that contradict an existing ADR and explain
why the decision should be reopened.
