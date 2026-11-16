# Issue tracker: GitHub

Issues and specs live in GitHub Issues in `amokrzycki/grajek`.
Use the `gh` CLI with `--repo amokrzycki/grajek`.

## Operations

- Create: `gh issue create --repo amokrzycki/grajek --title "..." --body-file <file>`.
- Read: `gh issue view <number> --repo amokrzycki/grajek --json number,title,body,labels,comments`.
- List: `gh issue list --repo amokrzycki/grajek --state open --json number,title,body,labels`.
- Comment: `gh issue comment <number> --repo amokrzycki/grajek --body-file <file>`.
- Labels: `gh issue edit <number> --repo amokrzycki/grajek --add-label "..."` or `--remove-label "..."`.
- Close: post the explanation as a comment, then run `gh issue close <number> --repo amokrzycki/grajek`.

Use GitHub native sub-issues and issue dependencies when supported.
Otherwise put `Part of #<parent>` and `Blocked by: #<number>` at the
top of the child issue body. Work is unblocked when every blocker is closed.

File issues only in this repository.

## Pull requests as a triage surface

**PRs as a request surface: no.**

## Skill conventions

“Publish to the issue tracker” means create a GitHub issue.
“Fetch the relevant ticket” means read the GitHub issue.

For wayfinding, use one map issue labelled `wayfinder:map`.
Link child tickets as sub-issues, or use a task list in the map and
`Part of #<map>` in each child. Label children `wayfinder:<type>`
(research, prototype, grilling, or task).
Choose the first open, unassigned child in map order with no open blockers.
Claim it by assigning yourself. Resolve it with a comment and closure,
then add the result and link to the map's Decisions-so-far.
