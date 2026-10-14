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

pub fn join_url(origin: &str, account: &str, enable_sync: bool, receipt: Option<&str>) -> Result<String, String> {
    let mut url = reqwest::Url::parse(origin).map_err(|_| "The browser app address is invalid.")?;
    url.set_path("/rank");
    url.query_pairs_mut().append_pair("join", "1").append_pair("from", "desktop").append_pair("account", account);
    add_join_receipt(&mut url, enable_sync, receipt)?;
    Ok(url.into())
}

pub(super) fn add_join_receipt(url: &mut reqwest::Url, enable_sync: bool, receipt: Option<&str>) -> Result<(), String> {
    if let Some(receipt) = receipt {
        if !super::browser_login::valid_flow_id(receipt) { return Err("Invalid leaderboard join receipt.".into()); }
        url.query_pairs_mut().append_pair("receipt", receipt);
    } else if enable_sync { return Err("A leaderboard join receipt is required for sync consent.".into()); }
    if enable_sync { url.query_pairs_mut().append_pair("sync", "enable"); }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn joining_binds_the_browser_to_the_native_account_without_url_injection() {
        let url = join_url(PRODUCTION_APP_ORIGIN, "account&token=secret#fragment", false, None).unwrap();
        let parsed = reqwest::Url::parse(&url).unwrap();
        assert_eq!(parsed.path(), "/rank");
        assert_eq!(parsed.fragment(), None);
        assert_eq!(parsed.query_pairs().collect::<Vec<_>>(), vec![
            ("join".into(), "1".into()), ("from".into(), "desktop".into()),
            ("account".into(), "account&token=secret#fragment".into()),
        ]);
    }
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
