use std::{fs, path::Path};

#[test]
fn manifest_dependencies_stay_transport_free_and_self_contained() {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path).unwrap_or_else(|error| {
        panic!(
            "could not read the crate manifest at {}: {error}",
            manifest_path.display()
        )
    });
    for dependency in dependency_names(&manifest) {
        if !matches!(dependency.as_str(), "serde_json" | "httpdate") {
            panic!(
                "forbidden dependency `{dependency}` is declared in {}",
                manifest_path.display()
            );
        }
    }
}

fn dependency_names(manifest: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_dependency_table = false;
    let mut in_dependency_subtable = false;

    for raw_line in manifest.lines() {
        let line = raw_line.split('#').next().unwrap_or_default().trim();
        if line.starts_with('[') && line.ends_with(']') {
            let section = line.trim_start_matches('[').trim_end_matches(']').trim();
            in_dependency_subtable = false;
            in_dependency_table = ["dependencies", "dev-dependencies", "build-dependencies"]
                .iter()
                .any(|name| {
                    let suffix = format!(".{name}");
                    section == *name || section.ends_with(suffix.as_str())
                });

            for name in ["dependencies", "dev-dependencies", "build-dependencies"] {
                let direct_prefix = format!("{name}.");
                if let Some(dependency) = section.strip_prefix(direct_prefix.as_str()) {
                    names.push(dependency.to_owned());
                    in_dependency_table = false;
                    in_dependency_subtable = true;
                    break;
                }

                let target_prefix = format!(".{name}.");
                if let Some(index) = section.rfind(target_prefix.as_str()) {
                    names.push(section[index + target_prefix.len()..].to_owned());
                    in_dependency_table = false;
                    in_dependency_subtable = true;
                    break;
                }
            }
            continue;
        }

        if !in_dependency_table && !in_dependency_subtable {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = unquote(key.trim());
        if in_dependency_table && key != "package" {
            names.push(key.to_owned());
        }
        if key == "package" || in_dependency_table {
            let package_value = if key == "package" {
                Some(unquote(value.trim()))
            } else {
                value
                    .split_once("package")
                    .and_then(|(_, suffix)| suffix.split_once('='))
                    .map(|(_, package)| unquote(package.trim()))
            };
            if let Some(package) = package_value.filter(|package| !package.is_empty()) {
                names.push(package.to_owned());
            }
        }
    }

    names
}

fn unquote(value: &str) -> &str {
    value.trim_matches(|character| character == '\"' || character == '\'')
}
