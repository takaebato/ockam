use crate::{debugger, ContextMode};
use crate::{relay::ProcessorRelay, Context};
use alloc::string::String;
use ockam_core::compat::{sync::Arc, vec::Vec};
use ockam_core::{
    Address, AddressAndMetadata, AddressMetadata, DenyAll, IncomingAccessControl, Mailboxes,
    OutgoingAccessControl, Processor, Result,
};

/// Start a [`Processor`]
///
/// Varying use-cases should use the builder API to customise the
/// underlying processor that is created.
pub struct ProcessorBuilder<P>
where
    P: Processor<Context = Context>,
{
    processor: P,
}

impl<P> ProcessorBuilder<P>
where
    P: Processor<Context = Context>,
{
    /// Create a new builder for a given Processor. Default AccessControl is DenyAll
    pub fn new(processor: P) -> Self {
        Self { processor }
    }
}

impl<P> ProcessorBuilder<P>
where
    P: Processor<Context = Context>,
{
    /// Worker with only one [`Address`]
    pub fn with_address(self, address: impl Into<Address>) -> ProcessorBuilderOneAddress<P> {
        ProcessorBuilderOneAddress {
            incoming_ac: Arc::new(DenyAll),
            outgoing_ac: Arc::new(DenyAll),
            processor: self.processor,
            address: address.into(),
            metadata: None,
        }
    }

    /// Worker with multiple [`Address`]es
    pub fn with_mailboxes(self, mailboxes: Mailboxes) -> ProcessorBuilderMultipleAddresses<P> {
        ProcessorBuilderMultipleAddresses {
            mailboxes,
            processor: self.processor,
            metadata_list: vec![],
        }
    }
}

pub struct ProcessorBuilderMultipleAddresses<P>
where
    P: Processor<Context = Context>,
{
    mailboxes: Mailboxes,
    processor: P,
    metadata_list: Vec<AddressAndMetadata>,
}

impl<P> ProcessorBuilderMultipleAddresses<P>
where
    P: Processor<Context = Context>,
{
    /// Mark the provided address as terminal
    pub fn terminal(self, address: impl Into<Address>) -> Self {
        self.terminal_with_attributes(address.into(), vec![])
    }

    /// Mark the provided address as terminal
    pub fn terminal_with_attributes(
        mut self,
        address: impl Into<Address>,
        attributes: Vec<(String, String)>,
    ) -> Self {
        let address = address.into();
        let metadata = self.metadata_list.iter_mut().find(|m| m.address == address);

        if let Some(metadata) = metadata {
            metadata.metadata.is_terminal = true;
            metadata.metadata.attributes = attributes;
        } else {
            self.metadata_list.push(AddressAndMetadata {
                address,
                metadata: AddressMetadata {
                    is_terminal: true,
                    attributes,
                },
            });
        }
        self
    }

    /// Adds metadata attribute for the provided address
    pub fn with_metadata_attribute(
        mut self,
        address: impl Into<Address>,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        let address = address.into();
        let metadata = self.metadata_list.iter_mut().find(|m| m.address == address);

        if let Some(metadata) = metadata {
            metadata
                .metadata
                .attributes
                .push((key.into(), value.into()));
        } else {
            self.metadata_list.push(AddressAndMetadata {
                address,
                metadata: AddressMetadata {
                    is_terminal: false,
                    attributes: vec![(key.into(), value.into())],
                },
            });
        }
        self
    }

    /// Consume this builder and start a new Ockam [`Processor`] from the given context
    pub async fn start(self, context: &Context) -> Result<()> {
        start(context, self.mailboxes, self.processor, self.metadata_list).await
    }
}

pub struct ProcessorBuilderOneAddress<P>
where
    P: Processor<Context = Context>,
{
    incoming_ac: Arc<dyn IncomingAccessControl>,
    outgoing_ac: Arc<dyn OutgoingAccessControl>,
    address: Address,
    processor: P,
    metadata: Option<AddressAndMetadata>,
}

impl<P> ProcessorBuilderOneAddress<P>
where
    P: Processor<Context = Context>,
{
    /// Mark the address as terminal
    pub fn terminal(self) -> Self {
        self.terminal_with_attributes(vec![])
    }

    /// Mark the address as terminal
    pub fn terminal_with_attributes(mut self, attributes: Vec<(String, String)>) -> Self {
        if let Some(metadata) = self.metadata.as_mut() {
            metadata.metadata.is_terminal = true;
            metadata.metadata.attributes = attributes;
        } else {
            self.metadata = Some(AddressAndMetadata {
                address: self.address.clone(),
                metadata: AddressMetadata {
                    is_terminal: true,
                    attributes,
                },
            });
        }
        self
    }

    /// Adds metadata attribute
    pub fn with_metadata_attribute(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        if let Some(metadata) = self.metadata.as_mut() {
            metadata
                .metadata
                .attributes
                .push((key.into(), value.into()));
        } else {
            self.metadata = Some(AddressAndMetadata {
                address: self.address.clone(),
                metadata: AddressMetadata {
                    is_terminal: false,
                    attributes: vec![(key.into(), value.into())],
                },
            });
        }
        self
    }

    /// Consume this builder and start a new Ockam [`Processor`] from the given context
    pub async fn start(self, context: &Context) -> Result<()> {
        start(
            context,
            Mailboxes::primary(self.address, self.incoming_ac, self.outgoing_ac),
            self.processor,
            self.metadata.map(|m| vec![m]).unwrap_or_default(),
        )
        .await
    }
}

impl<P> ProcessorBuilderOneAddress<P>
where
    P: Processor<Context = Context>,
{
    /// Set [`IncomingAccessControl`]
    pub fn with_incoming_access_control(
        mut self,
        incoming_access_control: impl IncomingAccessControl,
    ) -> Self {
        self.incoming_ac = Arc::new(incoming_access_control);
        self
    }

    /// Set [`IncomingAccessControl`]
    pub fn with_incoming_access_control_arc(
        mut self,
        incoming_access_control: Arc<dyn IncomingAccessControl>,
    ) -> Self {
        self.incoming_ac = incoming_access_control.clone();
        self
    }

    /// Set [`OutgoingAccessControl`]
    pub fn with_outgoing_access_control(
        mut self,
        outgoing_access_control: impl OutgoingAccessControl,
    ) -> Self {
        self.outgoing_ac = Arc::new(outgoing_access_control);
        self
    }

    /// Set [`OutgoingAccessControl`]
    pub fn with_outgoing_access_control_arc(
        mut self,
        outgoing_access_control: Arc<dyn OutgoingAccessControl>,
    ) -> Self {
        self.outgoing_ac = outgoing_access_control.clone();
        self
    }
}

/// Consume this builder and start a new Ockam [`Processor`] from the given context

pub async fn start<P>(
    context: &Context,
    mailboxes: Mailboxes,
    processor: P,
    metadata: Vec<AddressAndMetadata>,
) -> Result<()>
where
    P: Processor<Context = Context>,
{
    debug!(
        "Initializing ockam processor '{}' with access control in:{:?} out:{:?}",
        mailboxes.primary_address(),
        mailboxes.primary_mailbox().incoming_access_control(),
        mailboxes.primary_mailbox().outgoing_access_control(),
    );

    let addresses = mailboxes.addresses();

    // Pass it to the context
    let (ctx, sender, ctrl_rx) = context.new_with_mailboxes(mailboxes, ContextMode::Attached);

    debugger::log_inherit_context("PROCESSOR", context, &ctx);

    let router = context.router()?;
    router.start_processor(addresses, sender, metadata)?;

    // Then initialise the processor message relay
    ProcessorRelay::<P>::init(context.runtime(), processor, ctx, ctrl_rx);

    Ok(())
}
