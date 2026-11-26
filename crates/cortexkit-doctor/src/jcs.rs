use serde::Serialize;
use serde_json::Value;

const MAX_EXACT_INTEGER: u64 = 1 << 53;

/// Encode a doctor document as RFC 8785 canonical JSON, without reparsing floats.
pub(crate) fn to_vec<T: Serialize + ?Sized>(data: &T) -> serde_json::Result<Vec<u8>> {
    let value = serde_json::to_value(data)?;
    let mut out = Vec::new();
    write_value(&value, &mut out)?;
    Ok(out)
}

fn write_value(value: &Value, out: &mut Vec<u8>) -> serde_json::Result<()> {
    match value {
        Value::Number(number) => {
            if number.as_u64().is_some_and(|n| n <= MAX_EXACT_INTEGER)
                || number
                    .as_i64()
                    .is_some_and(|n| n.unsigned_abs() <= MAX_EXACT_INTEGER)
            {
                serde_json::to_writer(out, number)?;
            } else {
                // RFC 8785 uses IEEE 754 doubles and ECMAScript number formatting.
                let number = number.as_f64().expect("finite JSON number");
                out.extend_from_slice(ryu_js::Buffer::new().format_finite(number).as_bytes());
            }
        }
        Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i != 0 {
                    out.push(b',');
                }
                write_value(item, out)?;
            }
            out.push(b']');
        }
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_unstable_by(|(a, _), (b, _)| a.encode_utf16().cmp(b.encode_utf16()));
            out.push(b'{');
            for (i, (key, value)) in entries.into_iter().enumerate() {
                if i != 0 {
                    out.push(b',');
                }
                serde_json::to_writer(&mut *out, key)?;
                out.push(b':');
                write_value(value, out)?;
            }
            out.push(b'}');
        }
        // Strings and literals already have the desired spelling.
        _ => serde_json::to_writer(out, value)?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::to_vec;
    use serde_json::{json, Value};

    fn assert_matches_serde_jcs(value: &Value, label: &str) {
        let actual = to_vec(value).unwrap();
        let expected = serde_jcs::to_vec(value).unwrap();
        assert_eq!(
            std::str::from_utf8(&actual).unwrap(),
            std::str::from_utf8(&expected).unwrap(),
            "{label}"
        );
    }

    #[test]
    fn matches_serde_jcs_for_generated_values() {
        let controls = ('\0'..='\u{1f}').collect::<String>();
        let floats = [
            1e20,
            1e21,
            1e-6,
            1e-7,
            0.1,
            -0.0,
            5e-324,
            f64::MIN_POSITIVE,
            f64::MAX,
        ];
        for i in 0_u64..512 {
            let sample = f64::from_bits(i.wrapping_mul(0x9e37_79b9_7f4a_7c15));
            if !sample.is_finite() {
                continue;
            }
            let value = json!({
                "\u{e000}": [i64::MIN, i64::MAX, u64::MAX, (1_i64 << 53) - 1, 1_i64 << 53, (1_i64 << 53) + 1],
                "\u{10000}": {
                    "😀": [sample, floats[i as usize % floats.len()]],
                    "\u{fffd}": {"é": true, "e\u{301}": false, "a": null},
                    "\u{d7ff}": format!("{controls}\"\\/é\u{2028}\u{2029}😀"),
                    (format!("{controls}{i}")): [{}, []],
                },
            });
            assert_matches_serde_jcs(&value, &format!("sample {i}"));
        }
    }

    #[cfg(feature = "test-vectors")]
    #[test]
    fn matches_serde_jcs_for_doctor_fixtures() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors/doctor-v1");
        for name in [
            "inputs.json",
            "manifest.json",
            "report.json",
            "evicted-report.json",
            "timed-out-check.json",
        ] {
            let value = serde_json::from_slice(&std::fs::read(dir.join(name)).unwrap()).unwrap();
            assert_matches_serde_jcs(&value, name);
        }
    }
}
