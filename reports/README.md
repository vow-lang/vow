# Reports Retention Policy

`reports/` stores curated, reviewable evidence that is useful in the main
repository history. It is not the default destination for every generated run
output.

Committed report snapshots must:

- Use dated filenames: `YYYY-MM-DD-<topic>.md`.
- State their source or generator near the top of the file.
- State their retention class near the top of the file.

Use one of these retention classes:

- `current-baseline`: Keep at most one committed snapshot per report stream,
  such as `complexity-calibration`. When adding a newer snapshot for the same
  stream, delete or replace the previous one in the same PR unless a reviewer
  explicitly reclassifies it.
- `release-evidence`: Keep while the snapshot is referenced by a release,
  roadmap, publication, issue, or PR. Include the reference in the report.
- `temporary-review`: Allowed only while a PR is active. Remove it or
  reclassify it before merge.

Raw benchmark runs and bulk generated outputs belong in existing gitignored
locations such as `bench/results/`, `workflow/results/`, `verify-eval.out/`, or
an external artifact store. Do not put them in `reports/` just because they are
markdown or JSON.

Do not blanket-ignore `reports/`: curated snapshots are intentionally tracked.
