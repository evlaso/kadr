# Every check, one command each (docs: SPEC.ru.md, section 6).

# Everything that runs without a person: unit and golden tests.
test: deps unit golden

# Unit tests (the golden ones run separately).
unit:
    cargo test --release --workspace -- --skip golden

# The editor's rasteriser against tests/golden/cases; failures leave
# target/golden/<case>/{actual,diff}.png. UPDATE_GOLDEN=1 rewrites the expected pictures.
golden:
    cargo test --release -p kadr-raster --test golden

# Writes tests/golden/cases from crates/kadr-raster/tests/golden.rs's cases().
golden-cases:
    cargo test --release -p kadr-raster --test golden make_golden_cases -- --ignored

# The budgets of SPEC.ru.md section 2, on the live Hyprland session (the
# picker and an editor show for a moment). Writes target/bench.json; fails
# over budget, warns >15% worse than docs/perf.json.
bench *args:
    python3 scripts/bench.py {{args}}

# The crates' dependency rule (SPEC.ru.md section 3).
deps:
    scripts/check-deps.sh

# kadr end to end in a nested Hyprland of its own (scripts/nested.sh; needs a
# running Hyprland). Each test starts its own session.
e2e *args:
    cargo test --release --test e2e -- --ignored --test-threads=1 {{args}}
