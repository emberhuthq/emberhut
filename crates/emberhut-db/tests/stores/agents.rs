//! The agent store: the default agent, agents and their lists, approvers, policies, trusted
//! requesters and threads.
#![expect(clippy::unwrap_used, reason = "tests")]

use emberhut_core::RoomId;
use emberhut_core::agents::{
    Agent, AgentAccount, AgentError, AgentId, AgentTool, Channel, ChannelKind, Currency, SpendCap,
    TrustedRequester,
};
use emberhut_core::approvals::{
    ActionKind, Approver, ApproverId, ApproverRecord, Policy, PolicyError, SlackIdentity,
    WebhookTarget,
};
use emberhut_core::secret_link::SecretName;
use emberhut_db::{AgentStoreError, Database, ThreadPlace};

use crate::common::{at, database};

/// A new agent `id` in a room of its own.
pub(crate) async fn agent_in_own_room(db: &Database, id: &str) -> Agent {
    let room = db.rooms().create("Room 2", at(0)).await.unwrap();
    let agent = Agent {
        id: AgentId::parse(id).unwrap(),
        name: "Example helper".to_owned(),
        persona: Some("You help the example team.".to_owned()),
        principal: "Example Ltd".to_owned(),
        room: room.id,
        project: None,
        spend_cap: Some(SpendCap {
            cents: 10_000,
            currency: Currency::parse("EUR").unwrap(),
        }),
        enabled: true,
        worker_shell: false,
    };
    db.agents().create(&agent, at(0)).await.unwrap();
    agent
}

/// An external approver reached through Slack.
pub(crate) fn approver(id: &str) -> ApproverRecord {
    ApproverRecord {
        id: ApproverId::parse(id).unwrap(),
        name: "Example Lead".to_owned(),
        slack: Some(SlackIdentity {
            workspace: "T0EXAMPLE".to_owned(),
            user: "U0APPROVER".to_owned(),
        }),
        webhook: None,
        link: true,
    }
}

#[tokio::test]
async fn the_default_agent_is_there_from_the_start() {
    let (_dir, db) = database().await;
    let agents = db.agents();
    assert_eq!(agents.list().await.unwrap(), [Agent::main()]);
    assert_eq!(
        agents.get(&AgentId::MAIN).await.unwrap(),
        Some(Agent::main())
    );
    assert_eq!(agents.by_room(RoomId::MAIN).await.unwrap(), Agent::main());
    assert!(matches!(
        agents.delete(&AgentId::MAIN).await,
        Err(AgentStoreError::Invalid(AgentError::DefaultAgent(_)))
    ));
}

#[tokio::test]
async fn an_agent_is_made_changed_and_deleted() {
    let (_dir, db) = database().await;
    let agents = db.agents();
    let mut agent = agent_in_own_room(&db, "client-a").await;
    assert_eq!(agents.get(&agent.id).await.unwrap(), Some(agent.clone()));
    assert_eq!(agents.by_room(agent.room).await.unwrap(), agent);
    let other_room = db.rooms().create("Room 3", at(0)).await.unwrap();
    assert_eq!(
        agents.by_room(other_room.id).await.unwrap().id,
        AgentId::MAIN,
        "a room no agent owns is the default agent's"
    );
    let listed = agents.list().await.unwrap();
    assert_eq!(listed[0].id, AgentId::MAIN);
    assert_eq!(listed[1], agent);

    assert!(matches!(
        agents.create(&agent, at(1)).await,
        Err(AgentStoreError::AgentExists(_))
    ));

    agent.name = "Renamed helper".to_owned();
    agent.enabled = false;
    agent.spend_cap = None;
    agents.update(&agent, at(2)).await.unwrap();
    assert_eq!(agents.get(&agent.id).await.unwrap(), Some(agent.clone()));

    assert!(agents.delete(&agent.id).await.unwrap());
    assert!(!agents.delete(&agent.id).await.unwrap());
    assert_eq!(agents.get(&agent.id).await.unwrap(), None);
}

#[tokio::test]
async fn rooms_and_projects_are_checked() {
    let (_dir, db) = database().await;
    let agents = db.agents();
    let first = agent_in_own_room(&db, "client-a").await;
    let mut second = first.clone();
    second.id = AgentId::parse("client-b").unwrap();
    assert!(matches!(
        agents.create(&second, at(1)).await,
        Err(AgentStoreError::RoomTaken { agent, .. }) if agent == first.id
    ));
    second.room = RoomId::new(99);
    assert!(matches!(
        agents.create(&second, at(1)).await,
        Err(AgentStoreError::NoRoom(_))
    ));
    second.room = db.rooms().create("Room 3", at(0)).await.unwrap().id;
    second.project = Some(emberhut_core::task::ProjectId::new(42));
    assert!(matches!(
        agents.create(&second, at(1)).await,
        Err(AgentStoreError::NoProject(_))
    ));
    second.room = RoomId::MAIN;
    second.project = None;
    assert!(matches!(
        agents.create(&second, at(1)).await,
        Err(AgentStoreError::Invalid(AgentError::MainRoom))
    ));
    let mut main = Agent::main();
    main.name = "Renamed".to_owned();
    assert!(matches!(
        agents.update(&main, at(1)).await,
        Err(AgentStoreError::Invalid(AgentError::DefaultAgent(_)))
    ));
}

#[tokio::test]
async fn lists_are_replaced_whole_and_kept_in_order() {
    let (_dir, db) = database().await;
    let agents = db.agents();
    let agent = agent_in_own_room(&db, "client-a").await;
    let channels = vec![
        Channel {
            kind: ChannelKind::Slack,
            workspace: Some("T0EXAMPLE".to_owned()),
            address: "C0EXAMPLE".to_owned(),
            token_secret: Some(SecretName::parse("slack-example-bot").unwrap()),
        },
        Channel {
            kind: ChannelKind::WhatsApp,
            workspace: None,
            address: "15550100102@s.whatsapp.net".to_owned(),
            token_secret: None,
        },
    ];
    agents.replace_channels(&agent.id, &channels).await.unwrap();
    assert_eq!(agents.channels(&agent.id).await.unwrap(), channels);
    agents
        .replace_channels(&agent.id, &channels[1..])
        .await
        .unwrap();
    assert_eq!(agents.channels(&agent.id).await.unwrap(), &channels[1..]);

    let tools = vec![
        AgentTool::Server {
            name: "tasks".to_owned(),
        },
        AgentTool::Mcp {
            name: "example-admin".to_owned(),
            config: "/srv/agents/example-admin.json".to_owned(),
        },
    ];
    agents.replace_tools(&agent.id, &tools).await.unwrap();
    assert_eq!(agents.tools(&agent.id).await.unwrap(), tools);
    let refused = [AgentTool::Server {
        name: "slack_send".to_owned(),
    }];
    assert!(matches!(
        agents.replace_tools(&agent.id, &refused).await,
        Err(AgentStoreError::Invalid(AgentError::OwnerTool { .. }))
    ));
    assert_eq!(
        agents.tools(&agent.id).await.unwrap(),
        tools,
        "a refused list changes nothing"
    );
    assert!(matches!(
        agents.replace_tools(&AgentId::MAIN, &tools[..1]).await,
        Err(AgentStoreError::Invalid(AgentError::DefaultAgent(_)))
    ));

    let accounts = vec![AgentAccount {
        label: "admin".to_owned(),
        secret: SecretName::parse("example-admin-token").unwrap(),
    }];
    agents.replace_accounts(&agent.id, &accounts).await.unwrap();
    assert_eq!(agents.accounts(&agent.id).await.unwrap(), accounts);

    let nobody = AgentId::parse("nobody").unwrap();
    assert!(matches!(
        agents.replace_accounts(&nobody, &accounts).await,
        Err(AgentStoreError::NoAgent(_))
    ));

    agents.delete(&agent.id).await.unwrap();
    assert_eq!(agents.channels(&agent.id).await.unwrap(), []);
    assert_eq!(agents.tools(&agent.id).await.unwrap(), []);
    assert_eq!(agents.accounts(&agent.id).await.unwrap(), []);
}

#[tokio::test]
async fn policies_follow_the_fixed_core() {
    let (_dir, db) = database().await;
    let agents = db.agents();
    let agent = agent_in_own_room(&db, "client-a").await;
    let lead = approver("example-lead");
    let external = Approver::External(lead.id.clone());
    let post = Policy {
        kind: ActionKind::Post,
        approver: external.clone(),
    };
    assert!(matches!(
        agents
            .replace_policies(&agent.id, std::slice::from_ref(&post))
            .await,
        Err(AgentStoreError::NoApprover(_))
    ));
    agents.create_approver(&lead, at(0)).await.unwrap();
    let merge = Policy {
        kind: ActionKind::Merge,
        approver: Approver::Nobody,
    };
    agents
        .replace_policies(&agent.id, &[post.clone(), merge.clone()])
        .await
        .unwrap();
    assert_eq!(
        agents.policies(&agent.id).await.unwrap(),
        [post.clone(), merge]
    );
    assert_eq!(
        agents
            .approver_for(&agent.id, ActionKind::Post)
            .await
            .unwrap(),
        external
    );
    assert_eq!(
        agents
            .approver_for(&agent.id, ActionKind::Deploy)
            .await
            .unwrap(),
        Approver::Owner,
        "no row: the owner"
    );
    let escalation = Policy {
        kind: ActionKind::Payment,
        approver: external.clone(),
    };
    assert!(matches!(
        agents.replace_policies(&agent.id, &[escalation]).await,
        Err(AgentStoreError::Policy(PolicyError::FixedCore(
            ActionKind::Payment
        )))
    ));
    assert!(matches!(
        agents.replace_policies(&AgentId::MAIN, &[post]).await,
        Err(AgentStoreError::Policy(PolicyError::DefaultAgent))
    ));
    assert_eq!(
        agents.policy(&agent.id, ActionKind::Post).await.unwrap(),
        Some(external)
    );
}

#[tokio::test]
async fn approvers_are_kept_and_not_deleted_while_named() {
    let (_dir, db) = database().await;
    let agents = db.agents();
    let agent = agent_in_own_room(&db, "client-a").await;
    let mut lead = approver("example-lead");
    agents.create_approver(&lead, at(0)).await.unwrap();
    assert!(matches!(
        agents.create_approver(&lead, at(0)).await,
        Err(AgentStoreError::ApproverExists(_))
    ));
    lead.webhook = Some(WebhookTarget {
        url: "https://approvals.example.com/hook".parse().unwrap(),
        secret: SecretName::parse("example-webhook").unwrap(),
    });
    agents.update_approver(&lead, at(1)).await.unwrap();
    assert_eq!(agents.approver(&lead.id).await.unwrap(), Some(lead.clone()));
    assert_eq!(agents.approvers().await.unwrap(), [lead.clone()]);

    agents
        .replace_policies(
            &agent.id,
            &[Policy {
                kind: ActionKind::Post,
                approver: Approver::External(lead.id.clone()),
            }],
        )
        .await
        .unwrap();
    assert!(matches!(
        agents.delete_approver(&lead.id).await,
        Err(AgentStoreError::ApproverInUse(_))
    ));
    agents.replace_policies(&agent.id, &[]).await.unwrap();
    assert!(agents.delete_approver(&lead.id).await.unwrap());
    assert_eq!(agents.approvers().await.unwrap(), []);
    let mut unreachable = approver("other");
    unreachable.slack = None;
    assert!(matches!(
        agents.create_approver(&unreachable, at(0)).await,
        Err(AgentStoreError::Approver(_))
    ));
}

#[tokio::test]
async fn trusted_requesters_are_replaced_whole() {
    let (_dir, db) = database().await;
    let agents = db.agents();
    let agent = agent_in_own_room(&db, "client-a").await;
    let list = vec![
        TrustedRequester {
            requester: "Example Person".to_owned(),
            agent: None,
        },
        TrustedRequester {
            requester: "Example Person".to_owned(),
            agent: Some(agent.id.clone()),
        },
    ];
    agents.replace_trusted_requesters(&list).await.unwrap();
    assert_eq!(agents.trusted_requesters().await.unwrap(), list);
    agents.replace_trusted_requesters(&list[..1]).await.unwrap();
    assert_eq!(agents.trusted_requesters().await.unwrap(), &list[..1]);
    let unknown = [TrustedRequester {
        requester: "Someone".to_owned(),
        agent: Some(AgentId::parse("nobody").unwrap()),
    }];
    assert!(matches!(
        agents.replace_trusted_requesters(&unknown).await,
        Err(AgentStoreError::NoAgent(_))
    ));
}

#[tokio::test]
async fn threads_are_remembered_once_each() {
    let (_dir, db) = database().await;
    let agents = db.agents();
    let agent = agent_in_own_room(&db, "client-a").await;
    assert!(
        !agents
            .knows_thread(&agent.id, "C0EXAMPLE", "1800000000.000100")
            .await
            .unwrap()
    );
    let place = |thread, workspace| ThreadPlace {
        channel: "C0EXAMPLE",
        thread,
        workspace,
    };
    agents
        .touch_thread(
            &agent.id,
            place("1800000000.000100", Some("T0EXAMPLE")),
            Some("U0PERSON"),
            at(1),
        )
        .await
        .unwrap();
    agents
        .touch_thread(
            &agent.id,
            place("1800000000.000100", None),
            Some("U0OTHER"),
            at(3),
        )
        .await
        .unwrap();
    agents
        .touch_thread(&agent.id, place("1800000000.000200", None), None, at(2))
        .await
        .unwrap();
    assert!(
        agents
            .knows_thread(&agent.id, "C0EXAMPLE", "1800000000.000100")
            .await
            .unwrap()
    );
    let threads = agents.threads(&agent.id).await.unwrap();
    assert_eq!(threads.len(), 2, "one row per thread");
    assert_eq!(threads[0].thread, "1800000000.000100");
    assert_eq!(threads[0].last_seen, at(3));
    assert_eq!(threads[0].last_speaker.as_deref(), Some("U0OTHER"));
    assert_eq!(
        threads[0].workspace.as_deref(),
        Some("T0EXAMPLE"),
        "a known workspace is kept when none is given"
    );
    assert_eq!(threads[1].workspace, None);
    assert!(
        !agents
            .knows_thread(&AgentId::MAIN, "C0EXAMPLE", "1800000000.000100")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn an_agent_made_with_a_room_of_its_own_gets_one() {
    let (_dir, db) = database().await;
    let agents = db.agents();
    let asked = Agent {
        id: AgentId::parse("client-b").unwrap(),
        name: "Example desk".to_owned(),
        persona: None,
        principal: "Example Ltd".to_owned(),
        room: RoomId::MAIN,
        project: None,
        spend_cap: None,
        enabled: true,
        worker_shell: false,
    };
    let made = agents.create_in_new_room(&asked, at(0)).await.unwrap();
    assert_ne!(made.room, RoomId::MAIN);
    let room = db.rooms().get(made.room).await.unwrap().unwrap();
    assert_eq!(room.name, "Example desk");
    assert_eq!(agents.get(&made.id).await.unwrap(), Some(made.clone()));
    let rooms_before = db.rooms().list().await.unwrap().len();
    assert!(matches!(
        agents.create_in_new_room(&asked, at(1)).await,
        Err(AgentStoreError::AgentExists(_))
    ));
    let blank = Agent {
        id: AgentId::parse("client-c").unwrap(),
        name: " ".to_owned(),
        ..asked
    };
    assert!(matches!(
        agents.create_in_new_room(&blank, at(2)).await,
        Err(AgentStoreError::Invalid(AgentError::Blank("name")))
    ));
    assert_eq!(
        db.rooms().list().await.unwrap().len(),
        rooms_before,
        "a refused agent leaves no room behind"
    );
}

#[tokio::test]
async fn an_agent_that_did_anything_is_kept_and_its_empty_room_goes_with_it() {
    let (_dir, db) = database().await;
    let agents = db.agents();
    let busy = agent_in_own_room(&db, "client-a").await;
    let task = db
        .tasks()
        .create(
            emberhut_db::NewTask {
                title: "Its job".to_owned(),
                room: Some(busy.room),
                ..emberhut_db::NewTask::default()
            },
            emberhut_core::task::Who::Assistant,
            at(1),
        )
        .await
        .unwrap();
    assert_eq!(db.tasks().room_of(task.id).await.unwrap(), Some(busy.room));
    assert!(matches!(
        agents.delete(&busy.id).await,
        Err(AgentStoreError::HasTasks(_))
    ));
    let talked = agent_in_own_room(&db, "client-b").await;
    db.messages()
        .append(
            emberhut_core::message::NewMessage {
                room: talked.room,
                author: emberhut_core::message::Author::Event,
                text: "[Slack] Example person: hello".to_owned(),
                reply_to: None,
            },
            at(2),
        )
        .await
        .unwrap();
    assert!(matches!(
        agents.delete(&talked.id).await,
        Err(AgentStoreError::HasConversation(_))
    ));
    let idle = agents
        .create_in_new_room(
            &Agent {
                id: AgentId::parse("client-c").unwrap(),
                ..busy.clone()
            },
            at(3),
        )
        .await
        .unwrap();
    assert!(agents.delete(&idle.id).await.unwrap());
    assert_eq!(
        db.rooms().get(idle.room).await.unwrap(),
        None,
        "the room made with it goes with it, never to the owner"
    );
    assert!(db.rooms().get(busy.room).await.unwrap().is_some());
}

#[tokio::test]
async fn an_agent_given_an_existing_room_leaves_it_as_it_was() {
    let (_dir, db) = database().await;
    let agents = db.agents();
    let given = agent_in_own_room(&db, "client-a").await;
    assert!(agents.delete(&given.id).await.unwrap());
    assert!(
        db.rooms().get(given.room).await.unwrap().is_some(),
        "a room it was given, not made with it"
    );
    let mut moved = agents
        .create_in_new_room(
            &Agent {
                id: AgentId::parse("client-b").unwrap(),
                ..given.clone()
            },
            at(1),
        )
        .await
        .unwrap();
    let made = moved.room;
    moved.room = given.room;
    agents.update(&moved, at(2)).await.unwrap();
    assert!(agents.delete(&moved.id).await.unwrap());
    assert!(
        db.rooms().get(given.room).await.unwrap().is_some(),
        "moved to a room it was given"
    );
    assert!(db.rooms().get(made).await.unwrap().is_some());
}

#[tokio::test]
async fn a_subtask_and_a_finding_of_an_agents_task_are_the_agents() {
    let (_dir, db) = database().await;
    let agent = agent_in_own_room(&db, "client-a").await;
    let tasks = db.tasks();
    let scan = tasks
        .create(
            emberhut_db::NewTask {
                title: "Its scan".to_owned(),
                room: Some(agent.room),
                ..emberhut_db::NewTask::default()
            },
            emberhut_core::task::Who::Assistant,
            at(1),
        )
        .await
        .unwrap();
    for finding in [false, true] {
        let kid = tasks
            .create(
                emberhut_db::NewTask {
                    title: format!("Under it ({finding})"),
                    parent: Some(scan.id),
                    finding,
                    ..emberhut_db::NewTask::default()
                },
                emberhut_core::task::Who::Worker,
                at(2),
            )
            .await
            .unwrap();
        assert_eq!(
            tasks.room_of(kid.id).await.unwrap(),
            Some(agent.room),
            "finding: {finding}"
        );
    }
    let owners = tasks
        .create(
            emberhut_db::NewTask {
                title: "The owner's".to_owned(),
                ..emberhut_db::NewTask::default()
            },
            emberhut_core::task::Who::Person,
            at(3),
        )
        .await
        .unwrap();
    let kid = tasks
        .create(
            emberhut_db::NewTask {
                title: "Under the owner's".to_owned(),
                parent: Some(owners.id),
                ..emberhut_db::NewTask::default()
            },
            emberhut_core::task::Who::Person,
            at(4),
        )
        .await
        .unwrap();
    assert_eq!(tasks.room_of(kid.id).await.unwrap(), None);
}
