use super::{api::validate_origin, Period};

pub const PRODUCTION_API_ORIGIN: &str = "https://api.devclash.ai";
pub const PRODUCTION_APP_ORIGIN: &str = "https://app.devclash.ai";
pub const LEGACY_API_ORIGIN: &str = "https://devclash.ai";

pub fn app_origin(api_origin: &str, override_origin: Option<&str>) -> Result<String, String> {
    if let Some(origin) = override_origin {
        return validate_origin(origin).map_err(|_| {
            "The DevClash browser app address is invalid. Use an HTTPS origin or localhost.".into()
        });
    }
    if matches!(api_origin, PRODUCTION_API_ORIGIN | LEGACY_API_ORIGIN) {
        Ok(PRODUCTION_APP_ORIGIN.into())
    } else {
        // Preserve browser routes in local and custom single-service setups.
        Ok(api_origin.into())
    }
}

pub fn page_url(origin: &str, page: &str, period: Period) -> Result<String, String> {
    let path = match page {
        "rank" => format!("/rank?period={period:?}"),
        "profile" => "/profile".into(),
        _ => return Err("Unknown DevClash page.".into()),
    };
    Ok(format!("{origin}{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn production_pages_use_app_host_for_all_periods_and_both_api_hosts() {
        for api in [PRODUCTION_API_ORIGIN, LEGACY_API_ORIGIN] {
            let app = app_origin(api, None).unwrap();
            for period in [Period::Today, Period::Week, Period::Month] {
                assert_eq!(
                    page_url(&app, "rank", period).unwrap(),
                    format!("https://app.devclash.ai/rank?period={period:?}")
                );
            }
            assert_eq!(
                page_url(&app, "profile", Period::Today).unwrap(),
                "https://app.devclash.ai/profile"
            );
            assert!(page_url(&app, "https://untrusted.example", Period::Today).is_err());
        }
    }
    #[test]
    fn local_combined_and_explicit_split_origins_work_without_insecure_remote_urls() {
        assert_eq!(
            app_origin("http://127.0.0.1:1440", None).unwrap(),
            "http://127.0.0.1:1440"
        );
        assert_eq!(
            app_origin("http://127.0.0.1:1440", Some("http://127.0.0.1:1430/")).unwrap(),
            "http://127.0.0.1:1430"
        );
        for origin in [
            "http://remote.example",
            "https://user:secret@app.devclash.ai",
            "https://app.devclash.ai/path",
            "https://app.devclash.ai/?token=value",
            "",
        ] {
            assert!(app_origin(PRODUCTION_API_ORIGIN, Some(origin)).is_err());
        }
    }
}
