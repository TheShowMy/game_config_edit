# P6 performance baseline

The reproducible benchmark is `tests/performance_baseline.rs`. It is ignored by normal test runs because it generates a 100 MiB CSV and a workspace containing 2,000 sparse CSV files with a 2 GiB logical size.

Run it with:

```sh
cargo test --release --test performance_baseline -- --ignored --nocapture --test-threads=1
```

The npm tag release workflow runs this command on both supported platform runners before building or publishing packages.
It also records the packed size, unpacked size and file count of all three npm packages in the GitHub Actions job summary after validating their exact contents.

## Windows result

Measured on Windows 11 10.0.26200, Intel Core i7-12700H (20 logical processors), 31.6 GiB RAM, Release build:

| Measurement | Result | Requirement |
|---|---:|---:|
| Interactive window | 549 ms | <= 1 s |
| Workspace scan, 2,000 files / 2 GiB logical | 2 ms | first names <= 3 s |
| Path filter, 2,000 files | 154 us | <= 100 ms |
| Read 100 MiB CSV | 47 ms | supporting measurement |
| Parse 100 MiB / 493,593 data records | 589 ms | first table <= 3 s |
| Actual desktop first table | 1,204 ms | <= 3 s |
| Background type and warning analysis | 826 ms core / 1,930 ms desktop | remains interactive |
| Edit first data record | 34 ms | <= 100 ms UI blocking |
| Visible data rows at top / bottom | 39 / 33 | bounded DOM |
| Peak working set after opening and analyzing | 453.2 MiB | recorded, no fixed cap |
| Windows Release executable | 7.43 MiB | recorded, no fixed cap |

The machine used for this local measurement exceeds the minimum 4-core/16-GiB acceptance hardware. The workflow output is the authoritative Windows x64 and macOS arm64 release evidence for a tag.
