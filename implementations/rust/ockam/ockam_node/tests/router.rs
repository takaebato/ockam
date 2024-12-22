use ockam_core::{
    async_trait, route, Address, DenyAll, Mailbox, Mailboxes, Processor, Result, TransportType,
};
use ockam_node::{Context, NullWorker, ProcessorBuilder, WorkerBuilder};
use std::sync::Arc;

struct NullProcessor;

#[async_trait]
impl Processor for NullProcessor {
    type Context = Context;

    async fn process(&mut self, _ctx: &mut Context) -> Result<bool> {
        tokio::task::yield_now().await;
        Ok(true)
    }
}

#[ockam_macros::test]
async fn find_terminal_for_processor(context: &mut Context) -> Result<()> {
    ProcessorBuilder::new(NullProcessor {})
        .with_address("simple_processor")
        .start(context)
        .await?;

    assert!(context
        .find_terminal_address(route!["simple_processor", "non-existing"])?
        .is_none());

    ProcessorBuilder::new(NullProcessor {})
        .with_address("terminal_processor")
        .terminal()
        .start(context)
        .await?;

    assert_eq!(
        context
            .find_terminal_address(route![
                "simple_worker",
                "terminal_processor",
                "non-existing"
            ])?
            .unwrap()
            .address,
        "terminal_processor".into()
    );

    context.stop().await
}

#[ockam_macros::test]
async fn find_terminal_for_processor_alias(context: &mut Context) -> Result<()> {
    ProcessorBuilder::new(NullProcessor {})
        .with_mailboxes(Mailboxes::new(
            Mailbox::new("main", Arc::new(DenyAll), Arc::new(DenyAll)),
            vec![Mailbox::new("alias", Arc::new(DenyAll), Arc::new(DenyAll))],
        ))
        .terminal("alias")
        .start(context)
        .await?;

    assert!(context
        .find_terminal_address(route!["main", "non-existing"])?
        .is_none());

    assert_eq!(
        context
            .find_terminal_address(route!["main", "alias", "other"])?
            .unwrap()
            .address,
        "alias".into()
    );

    context.stop_address("main")?;
    ockam_node::compat::tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    assert!(context
        .find_terminal_address(route!["main", "alias", "other"])?
        .is_none());

    context.stop().await
}

#[ockam_macros::test]
async fn provide_and_read_processor_address_metadata(context: &mut Context) -> Result<()> {
    ProcessorBuilder::new(NullProcessor {})
        .with_address("processor_address")
        .with_metadata_attribute("TEST_KEY", "TEST_VALUE")
        .with_metadata_attribute("TEST_KEY_2", "TEST_VALUE_2")
        .start(context)
        .await?;

    let meta = context.get_metadata("processor_address")?.unwrap();

    assert!(!meta.is_terminal);

    assert_eq!(
        meta.attributes,
        vec![
            ("TEST_KEY".to_string(), "TEST_VALUE".to_string()),
            ("TEST_KEY_2".to_string(), "TEST_VALUE_2".to_string())
        ]
    );

    assert_eq!(context.get_metadata("non-existing-worker")?, None);

    context.stop_address("processor_address")?;
    ockam_node::compat::tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    assert_eq!(context.get_metadata("processor_address")?, None);

    context.stop().await
}

#[ockam_macros::test]
async fn find_terminal_for_worker(context: &mut Context) -> Result<()> {
    WorkerBuilder::new(NullWorker {})
        .with_address("simple_worker")
        .start(context)
        .await?;

    assert!(context
        .find_terminal_address(route!["simple_worker", "non-existing"])?
        .is_none());

    WorkerBuilder::new(NullWorker {})
        .with_address("terminal_worker")
        .terminal()
        .start(context)
        .await?;

    assert_eq!(
        context
            .find_terminal_address(route!["simple_worker", "terminal_worker", "non-existing"])?
            .unwrap()
            .address,
        "terminal_worker".into()
    );

    let remote = Address::new_with_string(TransportType::new(1), "127.0.0.1");
    assert!(context
        .find_terminal_address(route![
            "simple_worker",
            remote,
            "terminal_worker",
            "non-existing"
        ])
        .is_err());

    context.stop_address("terminal_worker")?;
    ockam_node::compat::tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    assert_eq!(
        context.find_terminal_address(route!["terminal_worker"])?,
        None
    );

    context.stop().await
}

#[ockam_macros::test]
async fn find_terminal_for_worker_alias(context: &mut Context) -> Result<()> {
    WorkerBuilder::new(NullWorker {})
        .with_mailboxes(Mailboxes::new(
            Mailbox::new("main", Arc::new(DenyAll), Arc::new(DenyAll)),
            vec![Mailbox::new("alias", Arc::new(DenyAll), Arc::new(DenyAll))],
        ))
        .terminal("alias")
        .start(context)
        .await?;

    assert!(context
        .find_terminal_address(route!["main", "non-existing"])?
        .is_none());

    assert_eq!(
        context
            .find_terminal_address(route!["main", "alias", "other"])?
            .unwrap()
            .address,
        "alias".into()
    );

    context.stop_address("main")?;
    ockam_node::compat::tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    assert!(context
        .find_terminal_address(route!["main", "alias", "other"])?
        .is_none());

    context.stop().await
}

#[ockam_macros::test]
async fn provide_and_read_address_metadata(context: &mut Context) -> Result<()> {
    WorkerBuilder::new(NullWorker {})
        .with_address("worker_address")
        .with_metadata_attribute("TEST_KEY", "TEST_VALUE")
        .with_metadata_attribute("TEST_KEY_2", "TEST_VALUE_2")
        .start(context)
        .await?;

    let meta = context.get_metadata("worker_address")?.unwrap();

    assert!(!meta.is_terminal);

    assert_eq!(
        meta.attributes,
        vec![
            ("TEST_KEY".to_string(), "TEST_VALUE".to_string()),
            ("TEST_KEY_2".to_string(), "TEST_VALUE_2".to_string())
        ]
    );

    assert_eq!(context.get_metadata("non-existing-worker")?, None);

    context.stop_address("worker_address")?;
    ockam_node::compat::tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    assert_eq!(context.get_metadata("worker_address")?, None);

    context.stop().await
}

#[ockam_macros::test]
async fn provide_and_read_address_metadata_worker_alias(context: &mut Context) -> Result<()> {
    WorkerBuilder::new(NullWorker {})
        .with_mailboxes(Mailboxes::new(
            Mailbox::new("main", Arc::new(DenyAll), Arc::new(DenyAll)),
            vec![Mailbox::new("alias", Arc::new(DenyAll), Arc::new(DenyAll))],
        ))
        .with_metadata_attribute("main", "TEST_KEY", "TEST_VALUE")
        .with_metadata_attribute("alias", "TEST_KEY_2", "TEST_VALUE_2")
        .start(context)
        .await?;

    let meta = context.get_metadata("alias")?.unwrap();

    assert!(!meta.is_terminal);

    assert_eq!(
        meta.attributes,
        vec![("TEST_KEY_2".to_string(), "TEST_VALUE_2".to_string())]
    );

    context.stop_address("main")?;
    ockam_node::compat::tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    assert_eq!(context.get_metadata("alias")?, None);

    context.stop().await
}
