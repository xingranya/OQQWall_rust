use oqqwall_rust_core::decide::decide;
use oqqwall_rust_core::event::{DraftEvent, RenderEvent, RenderFormat, SessionEvent};
use oqqwall_rust_core::{
    Command, CoreConfig, Event, EventEnvelope, GroupConfig, Id128, IngressAttachment,
    IngressCommand, IngressMessage, MediaKind, MediaReference, StateView, TickCommand,
};

fn wrap(event: Event, id: u128) -> EventEnvelope {
    EventEnvelope {
        id: Id128(id),
        ts_ms: 0,
        actor: Id128(0),
        correlation_id: None,
        event,
    }
}

#[test]
fn tick_closes_session_and_creates_draft() {
    let mut config = CoreConfig::default();
    config.default_process_waittime_ms = 1_000;
    config.groups.insert(
        "group-a".to_string(),
        GroupConfig {
            group_id: "group-a".to_string(),
            ..Default::default()
        },
    );

    let ingress = IngressCommand {
        profile_id: "bot".to_string(),
        chat_id: "chat".to_string(),
        user_id: "user".to_string(),
        group_id: "group-a".to_string(),
        platform_msg_id: "msg-1".to_string(),
        message: IngressMessage {
            text: "hello".to_string(),
            attachments: vec![IngressAttachment {
                kind: MediaKind::Image,
                name: None,
                reference: MediaReference::RemoteUrl {
                    url: "http://example.com/img.png".to_string(),
                },
            }],
        },
        received_at_ms: 1_000,
    };

    let mut state = StateView::default();
    let events = decide(&state, &Command::Ingress(ingress), &config);
    for (idx, event) in events.into_iter().enumerate() {
        state = state.reduce(&wrap(event, idx as u128 + 1));
    }

    let tick = TickCommand {
        now_ms: 3_000,
        tz_offset_minutes: 0,
    };
    let tick_events = decide(&state, &Command::Tick(tick), &config);
    assert_eq!(tick_events.len(), 3);

    let mut saw_close = false;
    let mut saw_draft = false;
    let mut saw_render = false;

    for event in tick_events {
        match event {
            Event::Session(SessionEvent::Closed { .. }) => saw_close = true,
            Event::Draft(DraftEvent::PostDraftCreated { group_id, ingress_ids, .. }) => {
                assert_eq!(group_id, "group-a");
                assert_eq!(ingress_ids.len(), 1);
                saw_draft = true;
            }
            Event::Render(RenderEvent::RenderRequested { format, .. }) => {
                assert_eq!(format, RenderFormat::Svg);
                saw_render = true;
            }
            _ => {}
        }
    }

    assert!(saw_close);
    assert!(saw_draft);
    assert!(saw_render);
}
