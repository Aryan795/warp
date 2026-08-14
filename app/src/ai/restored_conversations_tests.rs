use super::*;
use crate::persistence::model::{AgentConversation, AgentConversationRecord};

fn persisted_conversation(conversation_id: AIConversationId) -> AgentConversation {
    let task_id = format!("task-{conversation_id}");
    AgentConversation {
        conversation: AgentConversationRecord {
            id: 0,
            conversation_id: conversation_id.to_string(),
            conversation_data: r#"{"server_conversation_token":null}"#.to_string(),
            last_modified_at: chrono::NaiveDateTime::default(),
            summary: None,
        },
        tasks: vec![warp_multi_agent_api::Task {
            id: task_id,
            messages: vec![],
            dependencies: None,
            description: "Test conversation".to_string(),
            summary: String::new(),
            server_data: String::new(),
        }],
    }
}

fn ai_conversation(conversation_id: AIConversationId) -> AIConversation {
    convert_persisted_conversation_to_ai_conversation_with_metadata(persisted_conversation(
        conversation_id,
    ))
    .expect("test conversation should convert")
}

#[test]
fn take_conversation_hands_out_each_conversation_at_most_once() {
    let conversation_id = AIConversationId::new();
    let mut store =
        RestoredAgentConversations::new_seeded(vec![persisted_conversation(conversation_id)]);

    assert!(store.take_conversation(&conversation_id).is_some());
    assert!(
        store.take_conversation(&conversation_id).is_none(),
        "a taken conversation must not be handed out again"
    );
    assert!(
        store.get_conversation(&conversation_id).is_none(),
        "a taken conversation must not be readable either"
    );
}

#[test]
fn failed_take_does_not_consume_the_restore_opportunity() {
    let conversation_id = AIConversationId::new();
    // No seed and no backing database: the first take fails to load.
    let mut store = RestoredAgentConversations::new_seeded(vec![]);
    assert!(store.take_conversation(&conversation_id).is_none());

    // Once the conversation becomes available (e.g. the earlier failure was
    // transient), a retry must still succeed — a failed load must not have
    // marked the ID as taken.
    store
        .conversations
        .insert(conversation_id, ai_conversation(conversation_id));
    assert!(
        store.take_conversation(&conversation_id).is_some(),
        "a failed load must not permanently consume the restore"
    );
    assert!(store.take_conversation(&conversation_id).is_none());
}

#[test]
fn taken_and_unknown_conversations_are_not_restored_into_a_pane() {
    let conversation_id = AIConversationId::new();
    let mut store =
        RestoredAgentConversations::new_seeded(vec![persisted_conversation(conversation_id)]);

    assert!(store.should_restore_into_pane(&conversation_id));
    assert!(store.take_conversation(&conversation_id).is_some());
    assert!(
        !store.should_restore_into_pane(&conversation_id),
        "a conversation already handed out must not be restored again"
    );

    // Neither cached nor loadable: nothing to restore, and nothing cached.
    let unknown_id = AIConversationId::new();
    assert!(!store.should_restore_into_pane(&unknown_id));
    assert_eq!(store.cached_conversation_count(), 0);
}

#[cfg(feature = "local_fs")]
mod db_backed {
    use diesel::Connection as _;
    use diesel_migrations::MigrationHarness as _;
    use warp_multi_agent_api as api;

    use super::*;
    use crate::persistence::agent::upsert_agent_conversation_for_test;

    fn test_connection() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:")
            .expect("in-memory sqlite connection should open");
        conn.run_pending_migrations(::persistence::MIGRATIONS)
            .expect("migrations should run");
        conn
    }

    fn message(task_id: &str, id: &str, message: api::message::Message) -> api::Message {
        api::Message {
            id: id.to_string(),
            task_id: task_id.to_string(),
            message: Some(message),
            ..Default::default()
        }
    }

    fn user_query_message(task_id: &str) -> api::Message {
        message(
            task_id,
            &format!("{task_id}-user-query"),
            api::message::Message::UserQuery(api::message::UserQuery {
                query: "Initial query".to_string(),
                ..Default::default()
            }),
        )
    }

    fn auto_code_diff_message(task_id: &str) -> api::Message {
        message(
            task_id,
            &format!("{task_id}-auto-code-diff"),
            api::message::Message::SystemQuery(api::message::SystemQuery {
                context: None,
                r#type: Some(api::message::system_query::Type::AutoCodeDiff(
                    api::message::AutoCodeDiff {
                        query: "diff".to_string(),
                    },
                )),
            }),
        )
    }

    fn root_task(task_id: &str, messages: Vec<api::Message>) -> api::Task {
        api::Task {
            id: task_id.to_string(),
            description: String::new(),
            dependencies: None,
            messages,
            summary: String::new(),
            server_data: String::new(),
        }
    }

    /// Writes a conversation through the normal upsert path and returns the
    /// database it lives in.
    fn connection_with_conversation(
        conversation_id: AIConversationId,
        messages: Vec<api::Message>,
    ) -> SqliteConnection {
        let mut conn = test_connection();
        let task_id = format!("task-{conversation_id}");
        let messages = messages
            .into_iter()
            .map(|mut message| {
                message.task_id = task_id.clone();
                message
            })
            .collect();
        upsert_agent_conversation_for_test(
            &mut conn,
            &conversation_id.to_string(),
            [&root_task(&task_id, messages)],
        );
        conn
    }

    fn store_with_conversation(
        conversation_id: AIConversationId,
        messages: Vec<api::Message>,
    ) -> RestoredAgentConversations {
        RestoredAgentConversations::new_with_db_connection(connection_with_conversation(
            conversation_id,
            messages,
        ))
    }

    fn delete_task_rows(conn: &mut SqliteConnection, conversation: &str) {
        use diesel::prelude::*;

        use crate::persistence::schema::agent_tasks::dsl::*;
        diesel::delete(agent_tasks.filter(conversation_id.eq(conversation)))
            .execute(conn)
            .expect("task rows should delete");
    }

    fn clear_summary(conn: &mut SqliteConnection, conversation: &str) {
        use diesel::prelude::*;

        use crate::persistence::schema::agent_conversations::dsl::*;
        diesel::update(agent_conversations.filter(conversation_id.eq(conversation)))
            .set(summary.eq(None::<String>))
            .execute(conn)
            .expect("summary reset should succeed");
    }

    /// The filter decision must match what evaluating the loaded conversation
    /// would say, and must be reached without retaining anything.
    fn assert_filter_decision(messages: Vec<api::Message>, expected: bool) {
        let conversation_id = AIConversationId::new();

        let mut store = store_with_conversation(conversation_id, messages.clone());
        assert_eq!(
            store.should_restore_into_pane(&conversation_id),
            expected,
            "summary-backed decision disagrees with the expectation"
        );

        // The same decision reached from the fully loaded conversation, i.e.
        // the pre-change behavior this must stay equivalent to.
        let mut loading_store = store_with_conversation(conversation_id, messages);
        let loaded = loading_store
            .get_conversation(&conversation_id)
            .expect("conversation should load");
        let from_loaded = loaded.all_tasks().next().is_some() && !loaded.is_entirely_passive();
        assert_eq!(
            from_loaded, expected,
            "the loaded conversation disagrees with the expectation"
        );
    }

    #[test]
    fn filter_matches_loaded_behavior_for_a_conversation_with_a_user_query() {
        assert_filter_decision(vec![user_query_message("root")], true);
    }

    #[test]
    fn filter_matches_loaded_behavior_for_a_message_less_root_task() {
        assert_filter_decision(vec![], true);
    }

    #[test]
    fn filter_matches_loaded_behavior_for_an_entirely_passive_conversation() {
        assert_filter_decision(vec![auto_code_diff_message("root")], false);
    }

    #[test]
    fn filter_matches_loaded_behavior_for_a_passive_conversation_the_user_continued() {
        assert_filter_decision(
            vec![auto_code_diff_message("root"), user_query_message("root")],
            true,
        );
    }

    /// The point of the summary-backed path: a conversation the filter rejects
    /// must never be loaded into memory, let alone kept there.
    #[test]
    fn rejected_conversations_are_never_cached() {
        let conversation_id = AIConversationId::new();
        let mut store =
            store_with_conversation(conversation_id, vec![auto_code_diff_message("root")]);

        assert!(!store.should_restore_into_pane(&conversation_id));
        assert_eq!(
            store.cached_conversation_count(),
            0,
            "a conversation that failed the filter must not be retained"
        );
    }

    /// A conversation that passes is cached exactly once so the imminent
    /// `take_conversation` reuses the load, and taking it releases the payload.
    #[test]
    fn accepted_conversations_are_released_once_taken() {
        let conversation_id = AIConversationId::new();
        let mut store = store_with_conversation(conversation_id, vec![user_query_message("root")]);

        assert!(store.should_restore_into_pane(&conversation_id));
        assert!(store.take_conversation(&conversation_id).is_some());
        assert_eq!(store.cached_conversation_count(), 0);
    }

    /// Pins that the decision actually comes out of the `summary` column rather
    /// than out of the task payloads — which is the whole fix, and which every
    /// other test here would keep passing without.
    ///
    /// A passive conversation with no `agent_tasks` rows is the shape where the
    /// two paths disagree: the summary says "entirely passive" and rejects,
    /// while a fallback load finds no tasks, synthesizes an empty root, and
    /// accepts. The second half clears the summary to show the fallback really
    /// does reach the opposite answer, so the first assertion is discriminating
    /// rather than vacuous.
    #[test]
    fn the_filter_answers_from_the_summary_not_from_the_task_rows() {
        let conversation_id = AIConversationId::new();
        let mut conn =
            connection_with_conversation(conversation_id, vec![auto_code_diff_message("root")]);
        delete_task_rows(&mut conn, &conversation_id.to_string());

        let mut store = RestoredAgentConversations::new_with_db_connection(conn);
        assert!(
            !store.should_restore_into_pane(&conversation_id),
            "the filter must be answered from the summary column alone"
        );
        assert_eq!(store.cached_conversation_count(), 0);

        let conversation_id = AIConversationId::new();
        let mut conn =
            connection_with_conversation(conversation_id, vec![auto_code_diff_message("root")]);
        delete_task_rows(&mut conn, &conversation_id.to_string());
        clear_summary(&mut conn, &conversation_id.to_string());

        let mut store = RestoredAgentConversations::new_with_db_connection(conn);
        assert!(
            store.should_restore_into_pane(&conversation_id),
            "with no summary to read, the fallback load must run and reach the other answer"
        );
    }
}
