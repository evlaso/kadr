# Every check, one command each (docs: SPEC.ru.md, section 6).

# Everything that runs without a person: unit and golden tests.
test: unit golden

# Unit tests (the golden ones run separately).
unit:
    cargo test --release -- --skip golden

# The editor's rasteriser against tests/golden/cases; failures leave
# target/golden/<case>/{actual,diff}.png. UPDATE_GOLDEN=1 rewrites the expected pictures.
golden:
    cargo test --release golden::golden

# Writes tests/golden/cases from editor/golden.rs's cases().
golden-cases:
    cargo test --release make_golden_cases -- --ignored
