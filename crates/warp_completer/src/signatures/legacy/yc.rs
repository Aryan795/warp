use itertools::Itertools;
use warp_command_signatures::{
    Argument, ArgumentType, CommandBuilder, CommandSignatureGenerators, DynamicCompletionData,
    Generator, GeneratorName, GeneratorResults, IsArgumentOptional, Opt, ParserDirectives,
    Priority, Signature, Suggestion,
};
use warp_util::path::ShellFamily;

const COMPLETION_GENERATOR: &str = "yc_builtin_completion";

pub(super) fn signature() -> Signature {
    Signature {
        name: "yc".to_owned(),
        alias_generator: None,
        description: Some("Command line interface for Yandex Cloud".to_owned()),
        arguments: None,
        subcommands: Some(root_subcommands()),
        options: Some(global_options()),
        priority: Priority::default(),
        parser_directives: ParserDirectives::default(),
    }
}

pub(super) fn dynamic_completion_data_entry() -> (String, DynamicCompletionData) {
    CommandSignatureGenerators::new("yc")
        .add_generator(
            COMPLETION_GENERATOR,
            Generator::command_from_tokens(yc_completion_command, yc_completion_post_process),
        )
        .into()
}

fn yc_completion_command(
    tokens: &[&str],
    has_trailing_whitespace: bool,
    env_vars: &[String],
) -> CommandBuilder {
    let completed_token_count = tokens
        .len()
        .saturating_sub(1)
        .saturating_sub(usize::from(!has_trailing_whitespace));

    let mut command_parts = env_vars.to_vec();
    command_parts.push("yc".to_owned());
    command_parts.push("__completeNoDesc".to_owned());
    command_parts.extend(
        tokens
            .iter()
            .skip(1)
            .take(completed_token_count)
            .map(|token| ShellFamily::Posix.escape(token).into_owned()),
    );
    command_parts.push(ShellFamily::Posix.escape("").into_owned());

    CommandBuilder::single_command_and_ignore_stderr(command_parts.iter().join(" "))
}
fn completion_argument() -> Argument {
    Argument {
        display_name: Some("command|argument".to_owned()),
        description: Some("Yandex Cloud command or argument".to_owned()),
        is_variadic: true,
        argument_types: vec![ArgumentType::Generator(GeneratorName::new(
            COMPLETION_GENERATOR,
        ))],
        optional: IsArgumentOptional::Required,
        is_command: false,
        skip_generator_validation: true,
    }
}

fn yc_completion_post_process(output: &str) -> GeneratorResults {
    output
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty() && !line.starts_with(':') && !line.starts_with("Completion")
        })
        .filter_map(|line| line.split('\t').next())
        .map(Suggestion::new)
        .collect_ordered_results()
}

fn global_options() -> Vec<Opt> {
    [
        option_with_argument(
            &["--profile"],
            "Set the custom configuration file",
            "PROFILE",
            None,
        ),
        flag(&["--debug"], "Debug logging"),
        flag(
            &["--debug-grpc"],
            "Debug gRPC logging for connection problems",
        ),
        flag(
            &["--no-user-output"],
            "Disable printing user intended output to stderr",
        ),
        option_with_argument(
            &["--retry"],
            "Set the number of gRPC retry attempts",
            "ATTEMPTS",
            None,
        ),
        option_with_argument(
            &["--cloud-id"],
            "Set the ID of the cloud to use",
            "CLOUD_ID",
            None,
        ),
        option_with_argument(
            &["--folder-id"],
            "Set the ID of the folder to use",
            "FOLDER_ID",
            None,
        ),
        option_with_argument(
            &["--folder-name"],
            "Set the name of the folder to use",
            "FOLDER_NAME",
            None,
        ),
        option_with_argument(
            &["--endpoint"],
            "Set the Cloud API endpoint",
            "ENDPOINT",
            None,
        ),
        option_with_argument(&["--token"], "Set the OAuth token to use", "TOKEN", None),
        option_with_argument(
            &["--impersonate-service-account-id"],
            "Set the ID of the service account to impersonate",
            "SERVICE_ACCOUNT_ID",
            None,
        ),
        flag(
            &["--no-browser"],
            "Disable opening browser for authentication",
        ),
        option_with_argument(
            &["--format"],
            "Set the output format",
            "FORMAT",
            Some(&["text", "yaml", "json", "json-rest"]),
        ),
        option_with_argument(
            &["--jq"],
            "Query to select values from the response using jq syntax",
            "EXPRESSION",
            None,
        ),
        flag(&["-h", "--help"], "Display help for the command"),
    ]
    .into()
}

fn flag(names: &[&str], description: &str) -> Opt {
    Opt {
        exact_string: names.iter().map(|name| (*name).to_owned()).collect(),
        description: Some(description.to_owned()),
        arguments: None,
        required: false,
        priority: Priority::default(),
    }
}

fn option_with_argument(
    names: &[&str],
    description: &str,
    display_name: &str,
    suggestions: Option<&[&str]>,
) -> Opt {
    Opt {
        exact_string: names.iter().map(|name| (*name).to_owned()).collect(),
        description: Some(description.to_owned()),
        arguments: Some(vec![Argument {
            display_name: Some(display_name.to_owned()),
            description: None,
            is_variadic: false,
            argument_types: suggestions
                .unwrap_or_default()
                .iter()
                .map(|suggestion| ArgumentType::Suggestion(Suggestion::new(*suggestion)))
                .collect(),
            optional: IsArgumentOptional::Required,
            is_command: false,
            skip_generator_validation: suggestions.is_none(),
        }]),
        required: false,
        priority: Priority::default(),
    }
}

fn root_subcommands() -> Vec<Signature> {
    root_command_suggestions()
        .iter()
        .map(|(name, description)| Signature {
            name: (*name).to_owned(),
            alias_generator: None,
            description: Some((*description).to_owned()),
            arguments: Some(vec![completion_argument()]),
            subcommands: None,
            options: Some(global_options()),
            priority: Priority::default(),
            parser_directives: ParserDirectives::default(),
        })
        .collect()
}

fn root_command_suggestions() -> &'static [(&'static str, &'static str)] {
    &[
        (
            "application-load-balancer",
            "Manage Yandex Application Load Balancer resources",
        ),
        ("audit-trails", "Manage Audit Trails resources"),
        ("backup", "Manage Yandex Cloud Backup resources"),
        ("baremetal", "Manage Baremetal resources"),
        ("cdn", "Manage CDN resources"),
        (
            "certificate-manager",
            "Manage Certificate Manager resources",
        ),
        ("cic", "Manage Interconnect resources"),
        ("cloud-registry", "Manage Cloud Registry resources"),
        ("cloudrouter", "Manage Cloud Router resources"),
        ("components", "Manage installed components"),
        ("compute", "Manage Yandex Compute Cloud resources"),
        ("config", "Set, view, and unset Yandex Cloud CLI properties"),
        ("container", "Manage Container resources"),
        ("dataproc", "Manage data processing clusters"),
        (
            "datatransfer",
            "Manage Data Transfer endpoints and transfers",
        ),
        ("desktops", "Manage Desktop resources"),
        ("dns", "Manage Yandex DNS resources"),
        ("help", "Help for any command"),
        ("iam", "Manage Yandex Identity and Access Manager resources"),
        ("init", "CLI initialization"),
        ("iot", "Manage Yandex IoT Core resources"),
        ("kms", "Manage Yandex Key Management Service resources"),
        ("load-balancer", "Manage Yandex Load Balancer resources"),
        ("lockbox", "Manage Yandex Lockbox resources"),
        ("logging", "Manage Yandex Cloud Logging resources"),
        ("managed-airflow", "Manage Airflow clusters"),
        ("managed-clickhouse", "Manage ClickHouse clusters"),
        ("managed-gitlab", "Manage GitLab resources"),
        (
            "managed-greenplum",
            "Manage Greenplum and Cloudberry clusters",
        ),
        ("managed-kafka", "Manage Apache Kafka clusters"),
        ("managed-kubernetes", "Manage Kubernetes clusters"),
        ("managed-metastore", "Manage Metastore clusters"),
        ("managed-mongodb", "Manage MongoDB clusters"),
        ("managed-mysql", "Manage MySQL clusters"),
        ("managed-opensearch", "Manage OpenSearch clusters"),
        ("managed-postgresql", "Manage PostgreSQL clusters"),
        ("managed-redis", "Manage Redis clusters"),
        (
            "managed-sharded-postgresql",
            "Manage Sharded PostgreSQL clusters",
        ),
        ("managed-spark", "Manage Spark clusters"),
        ("managed-trino", "Manage Trino clusters"),
        ("managed-ytsaurus", "Manage YTsaurus clusters"),
        ("marketplace", "Manage Yandex Marketplace resources"),
        ("metadata-hub", "Manage Metadata Hub resources"),
        ("operation", "Manage operations"),
        (
            "organization-manager",
            "Manage Yandex Organization Manager resources",
        ),
        ("quota-manager", "Manage Yandex Quota Manager resources"),
        (
            "resource-manager",
            "Manage Yandex Resource Manager resources",
        ),
        ("serverless", "Manage Serverless resources"),
        ("smartcaptcha", "Manage SmartCaptcha resources"),
        ("smartwebsecurity", "Manage SmartWebSecurity resources"),
        ("storage", "Manage Yandex Object Storage resources"),
        ("version", "Display Yandex Cloud CLI version"),
        ("vpc", "Manage Yandex Virtual Private Cloud resources"),
        ("ydb", "Manage YDB databases"),
    ]
}

trait GeneratorResultsCollector: Iterator<Item = Suggestion> {
    fn collect_ordered_results(self) -> GeneratorResults
    where
        Self: Sized,
    {
        GeneratorResults {
            suggestions: self.collect(),
            is_ordered: true,
        }
    }
}

impl<T> GeneratorResultsCollector for T where T: Iterator<Item = Suggestion> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yc_completion_command_uses_completed_tokens_and_empty_query() {
        assert_eq!(
            yc_completion_command(&["yc"], true, &[]).build(warp_command_signatures::Shell::Posix),
            "yc __completeNoDesc '' 2>/dev/null"
        );
        assert_eq!(
            yc_completion_command(&["yc", "compute", "inst"], false, &[])
                .build(warp_command_signatures::Shell::Posix),
            "yc __completeNoDesc compute '' 2>/dev/null"
        );
        assert_eq!(
            yc_completion_command(&["yc", "compute"], true, &[])
                .build(warp_command_signatures::Shell::Posix),
            "yc __completeNoDesc compute '' 2>/dev/null"
        );
    }

    #[test]
    fn yc_completion_post_process_filters_metadata_and_descriptions() {
        let results = yc_completion_post_process(
            "compute\tManage compute resources\n:4\nCompletion ended with directive: ShellCompDirectiveNoFileComp\nconfig\n",
        );

        assert!(results.is_ordered);
        assert_eq!(
            results
                .suggestions
                .into_iter()
                .map(|suggestion| suggestion.exact_string)
                .collect::<Vec<_>>(),
            vec!["compute", "config"]
        );
    }
}
