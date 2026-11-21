# Contribution and Git conventions

The repository owner alone creates branches, commits, pull requests and merges. External assistants may inspect the repository in read-only mode and provide local patches.

## Branch names

`<branch_type>/<branch_name>` where the branch type is `feature` (not `feat`), `fix`, `refactor`, `docs`, `test`, `chore`, etc.

Example: `feature/cef-page-rendering`.

## Commit messages

Every commit has a header, body and footer:

```
feature(cef-page-rendering): Render Chromium pages inside Slint

Integrate a first offscreen Chromium renderer behind replaceable contracts.

- Implement the CEF process lifecycle and OSR paint callback.
- Bridge page frames and navigation events into the frontend.
- Add core tests and runtime acceptance documentation.

Stage: P0-CEF
```

Pull request titles and internal documentation are written in English. The owner is responsible for running the relevant gates and deciding when to merge.
