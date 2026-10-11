use super::*;
struct FullReads<R>(R);
impl<R: Read> Read for FullReads<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let mut used = 0;
        while used < buffer.len() {
            let count = self.0.read(&mut buffer[used..])?;
            if count == 0 {
                break;
            }
            used += count;
        }
        Ok(used)
    }
}

#[test]
fn malformed_large_strings_fail_without_formatting_the_input_into_an_error() {
    use serde_json::json;
    let project = with_run();
    let original: serde_json::Value = serde_json::from_slice(&bytes(&project)).unwrap();
    let mut cases = Vec::new();
    for path in [
        "/version",
        "/dataset/rows",
        "/dataset",
        "/dataset/columns",
        "/dataset/columns/0",
        "/dataset/columns/0/values",
        "/dataset/columns/0/id",
        "/dataset/columns/0/metadata/display_decimals",
        "/runs/0/analysis/describe/plots/0/reduced",
    ] {
        let mut value = original.clone();
        *value.pointer_mut(path).unwrap() = json!("__PAYLOAD__");
        let headroom = if path.ends_with("/id") || path.ends_with("/reduced") {
            96
        } else {
            56
        };
        cases.push((path, serde_json::to_string(&value).unwrap(), headroom));
    }
    let mut unknown = original.clone();
    unknown
        .as_object_mut()
        .unwrap()
        .insert("__PAYLOAD__".into(), json!(0));
    cases.push((
        "unknown field",
        serde_json::to_string(&unknown).unwrap(),
        96,
    ));
    let mut variant = original.clone();
    variant["runs"][0]["analysis"]["request"]["filter"]["predicates"] =
        json!([{"__PAYLOAD__":null}]);
    cases.push((
        "unknown variant",
        serde_json::to_string(&variant).unwrap(),
        96,
    ));
    let mut payload = original.clone();
    payload["runs"][0]["analysis"]["request"]["filter"]["predicates"] =
        json!([{"number":"__PAYLOAD__"}]);
    cases.push((
        "variant payload",
        serde_json::to_string(&payload).unwrap(),
        56,
    ));
    let mut escaped = original.clone();
    escaped["version"] = json!("__PAYLOAD__");
    let escaped = serde_json::to_string(&escaped)
        .unwrap()
        .replace("__PAYLOAD__", "__PAYLOAD__\\q");
    cases.push(("malformed escape", escaped, 56));
    let mut excessive = Vec::new();
    for (name, fixture, headroom_mib) in cases {
        let (prefix, suffix) = fixture.split_once("__PAYLOAD__").unwrap();
        let reader = std::io::Cursor::new(prefix.as_bytes())
            .chain(std::io::repeat(b'x').take(30 * 1024 * 1024))
            .chain(std::io::Cursor::new(suffix.as_bytes()));
        let control = JobControl::default();
        let (result, allocated) = allocations(|| {
            decode_with_budget(
                FullReads(reader),
                &control,
                &[],
                MAX_OWNED_BYTES - headroom_mib * 1024 * 1024,
            )
        });
        assert!(result.is_err(), "accepted {name}");
        // Input capacities total just under 64 MiB. Error construction must remain
        // small; this reader supplies the 30 MiB value without owning another copy.
        if allocated >= 65 * 1024 * 1024 {
            excessive.push((name, allocated));
        }
    }
    assert!(
        excessive.is_empty(),
        "oversized error allocations: {excessive:?}"
    );
}

#[test]
fn long_numeric_parser_scratch_is_refused_before_growth() {
    let fixture = String::from_utf8(altered(
        &with_run(),
        "/dataset/columns/1/values/0",
        serde_json::json!("__NUMBER__"),
    ))
    .unwrap();
    let (prefix, suffix) = fixture.split_once("\"__NUMBER__\"").unwrap();
    let reader = std::io::Cursor::new(prefix.as_bytes())
        .chain(std::io::Cursor::new(b"0."))
        .chain(std::io::repeat(b'1').take(30 * 1024 * 1024))
        .chain(std::io::Cursor::new(suffix.as_bytes()));
    let control = JobControl::default();
    let (result, allocated) = allocations(|| {
        decode_with_budget(
            FullReads(reader),
            &control,
            &[],
            MAX_OWNED_BYTES - 56 * 1024 * 1024,
        )
    });
    assert!(
        allocated < 65 * 1024 * 1024,
        "unreserved numeric scratch: {allocated} bytes"
    );
    assert!(matches!(
        result,
        Err(tessera::error::AppError::SizeLimit { .. })
    ));
}
