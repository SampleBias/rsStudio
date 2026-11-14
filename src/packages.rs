//! Curated R packages and safe parsing of the worker's package status response.
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Repository {
    Cran,
    Bioconductor,
    Bundled,
}
#[derive(Clone)]
pub struct Package {
    pub name: String,
    pub description: &'static str,
    pub repository: Repository,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub version: Option<String>,
    pub loaded: bool,
}

pub fn catalog() -> Vec<Package> {
    use Repository::{Bioconductor as Bio, Bundled, Cran};
    [
        ("tidyverse", "Data science collection", Cran),
        ("dplyr", "Data manipulation", Cran),
        ("ggplot2", "Statistical visualization", Cran),
        ("tidyr", "Data reshaping", Cran),
        ("data.table", "Fast data processing", Cran),
        ("readr", "Delimited files", Cran),
        ("stringr", "String manipulation", Cran),
        ("lubridate", "Dates and times", Cran),
        ("purrr", "Functional programming", Cran),
        ("tibble", "Modern data frames", Cran),
        ("caret", "Machine learning workflows", Cran),
        ("tidymodels", "Modeling ecosystem", Cran),
        ("randomForest", "Random forests", Cran),
        ("xgboost", "Gradient boosting", Cran),
        ("stats", "R's built-in statistics", Bundled),
        ("lme4", "Mixed-effects models", Cran),
        ("survival", "Survival analysis", Cran),
        ("broom", "Tidy statistical results", Cran),
        ("forecast", "Time series", Cran),
        ("shiny", "Interactive applications", Cran),
        ("plotly", "Interactive plots", Cran),
        ("BiocManager", "Bioconductor installer", Cran),
        ("DESeq2", "Differential expression", Bio),
        ("limma", "Gene expression analysis", Bio),
    ]
    .into_iter()
    .map(|(name, description, repository)| Package {
        name: name.into(),
        description,
        repository,
    })
    .collect()
}
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name.as_bytes()[0].is_ascii_alphabetic()
        && !name.ends_with('.')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.')
}
pub fn parse_status(text: &str, expected: &[Package]) -> Result<BTreeMap<String, Status>, String> {
    let mut statuses = BTreeMap::new();
    for line in text.lines() {
        let mut fields = line.split('\t');
        let (Some(name), Some(version), Some(loaded), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err("Invalid package status response".into());
        };
        if !expected.iter().any(|p| p.name == name)
            || statuses.contains_key(name)
            || (loaded != "0" && loaded != "1")
            || version.is_empty()
            || version.len() > 100
        {
            return Err("Invalid package status response".into());
        }
        let version = if version == "-" {
            None
        } else {
            Some(version.to_owned())
        };
        if version.is_none() && loaded == "1" {
            return Err("Invalid package status response".into());
        }
        statuses.insert(
            name.to_owned(),
            Status {
                version,
                loaded: loaded == "1",
            },
        );
    }
    if statuses.len() != expected.len() {
        return Err("Package status response is incomplete".into());
    }
    Ok(statuses)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_package_name_injection() {
        for name in [
            "",
            "../evil",
            "dplyr;quit()",
            "pkg\nother",
            ".hidden",
            "bad.",
            "pkg_name",
        ] {
            assert!(!valid_name(name));
        }
        for name in ["dplyr", "data.table", "DESeq2", "randomForest"] {
            assert!(valid_name(name));
        }
    }
    #[test]
    fn status_is_complete_and_validated() {
        let expected = vec![
            Package {
                name: "stats".into(),
                description: "",
                repository: Repository::Bundled,
            },
            Package {
                name: "dplyr".into(),
                description: "",
                repository: Repository::Cran,
            },
        ];
        let status = parse_status("stats\t4.5.0\t1\ndplyr\t-\t0\n", &expected).unwrap();
        assert!(status["stats"].loaded);
        assert_eq!(status["dplyr"].version, None);
        assert!(parse_status("stats\t4.5.0\t1\n", &expected).is_err());
        assert!(parse_status("stats\t4.5.0\t1\ndplyr\t-\t1\n", &expected).is_err());
    }
}
