# Reports Retention Policy

`reports/` stores curated, reviewable evidence that is useful in the main
repository history. It is not the default destination for every generated run
output.

Committed report snapshots must:

- Use dated filenames: `YYYY-MM-DD-<topic>.md`.
- State their source or generator near the top of the file.
- State their retention class near the top of the file.

Generators that write under `reports/` must enforce those three requirements
before they produce the snapshot.

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

## Complexity calibration

For local calibration runs, use the default output path:

```bash
python3 scripts/complexity_calibrate.py
```

This writes `reports.out/complexity-calibration.md`, which is ignored by git.

To intentionally commit a complexity calibration snapshot:

```bash
python3 scripts/complexity_calibrate.py --date YYYY-MM-DD --out reports/YYYY-MM-DD-complexity-calibration.md --retention-class current-baseline
```

The committed snapshot is part of the `complexity-calibration` stream. Keep only
the latest `current-baseline` snapshot for that stream unless a reviewer
reclassifies an older snapshot as `release-evidence`.

Do not blanket-ignore `reports/`: curated snapshots are intentionally tracked.
