use super::*;

#[test]
fn cli_uses_the_shared_api_default() {
    let command = Cli::command();
    let base_url = command
        .get_arguments()
        .find(|arg| arg.get_id() == "base_url")
        .unwrap();
    assert_eq!(
        base_url.get_default_values(),
        &[std::ffi::OsStr::new(api::DEFAULT_BASE_URL)]
    );
    assert_eq!(api::DEFAULT_BASE_URL, "https://jorik.gleckson.com");
}

#[test]
fn global_host_override_is_resolved_before_and_after_the_tui_subcommand() {
    for args in [
        ["jorik", "--base-url", "https://custom.example", "tui"],
        ["jorik", "tui", "--base-url", "https://custom.example"],
    ] {
        let matches = Cli::command().try_get_matches_from(args).unwrap();
        let cli = Cli::from_arg_matches(&matches).unwrap();
        assert_eq!(
            matches.value_source("base_url"),
            Some(ValueSource::CommandLine)
        );
        assert_eq!(
            resolve_base_url(
                "https://saved.example",
                &cli.base_url,
                matches.value_source("base_url")
            ),
            "https://custom.example"
        );
        assert!(matches!(cli.command, Commands::Tui { .. }));
    }
}

#[test]
fn saved_host_is_used_only_for_the_parser_default() {
    assert_eq!(
        resolve_base_url(
            "https://saved.example",
            api::DEFAULT_BASE_URL,
            Some(ValueSource::DefaultValue)
        ),
        "https://saved.example"
    );
    // Explicitly selecting the public default must override a saved host too.
    for source in [ValueSource::CommandLine, ValueSource::EnvVariable] {
        assert_eq!(
            resolve_base_url("https://saved.example", api::DEFAULT_BASE_URL, Some(source)),
            api::DEFAULT_BASE_URL
        );
        assert_eq!(
            resolve_base_url(
                "https://saved.example",
                "https://custom.example",
                Some(source)
            ),
            "https://custom.example"
        );
    }
}
