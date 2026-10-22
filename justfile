# Every check, one command each (docs: SPEC.ru.md, section 6).

# Everything that runs without a person: unit and golden tests.
test: unit golden

# Unit tests (the golden ones run separately).
unit:
    cargo test --profile fast --workspace -- --skip golden

# The editor's rasteriser against tests/golden/cases; failures leave
# target/golden/<case>/{actual,diff}.png. UPDATE_GOLDEN=1 rewrites the expected pictures.
golden:
    cargo test --profile fast -p kadr-raster --test golden

# Writes tests/golden/cases from crates/kadr-raster/tests/golden.rs's cases().
golden-cases:
    cargo test --profile fast -p kadr-raster --test golden make_golden_cases -- --ignored

# The budgets of SPEC.ru.md section 2, on the live Hyprland session (the
# picker and an editor show for a moment). Writes target/bench.json; fails
# over budget, warns >15% worse than docs/perf.json.
bench *args:
    python3 scripts/bench.py {{args}}
