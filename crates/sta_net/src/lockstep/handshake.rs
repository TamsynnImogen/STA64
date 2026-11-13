//! The host's side of the join handshake (docs/NET.md, section 5; D9, D10),
//! as a function of values so two in-process peers can be matched or refused
//! before a game starts. NET-03 calls it with what arrived over the wire;
//! CON-35 supplies the real [`RulesIdentity`].

use crate::proto::{
    first_difference, rules_parts, wire_entries, EngineId, Hello, ProtoError, Refusal,
    RulesAssembler, RulesIdentity,
};

/// Why [`admit`] did not admit a joiner.
#[derive(Debug, Clone, PartialEq)]
pub enum HandshakeError {
    /// The host sends this `Refuse` and closes the connection.
    Refused(Refusal),
    /// The joiner's rule entries broke a bound or the part order; on the
    /// wire, `Refuse(Protocol)`.
    Protocol(ProtoError),
}

impl HandshakeError {
    /// The refusal the host puts on the wire.
    pub fn refusal(&self) -> Refusal {
        match self {
            HandshakeError::Refused(r) => r.clone(),
            HandshakeError::Protocol(_) => Refusal::Protocol,
        }
    }
}

impl std::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandshakeError::Refused(Refusal::Engine { host }) => write!(
                f,
                "another build: host runs {} {} ({})",
                host.name, host.version, host.build
            ),
            HandshakeError::Refused(Refusal::RulesDiffer(d)) => write!(
                f,
                "rules differ: {} is {} here, {} there",
                d.key,
                d.host.as_deref().unwrap_or("absent"),
                d.joiner.as_deref().unwrap_or("absent")
            ),
            HandshakeError::Refused(r) => write!(f, "refused: {r:?}"),
            HandshakeError::Protocol(e) => write!(f, "malformed rule list: {e}"),
        }
    }
}

impl std::error::Error for HandshakeError {}

/// Decides whether `hello` may join a game hosted with `host_engine` and
/// `host_rules`. `joiner_rules` is asked for its entries only when the digests
/// differ (the `RulesRequest` / `Rules` exchange), and they are carried the
/// way the wire carries them ([`rules_parts`], [`RulesAssembler`]).
///
/// - Engines must match exactly ([`EngineId::matches`]); else
///   [`Refusal::Engine`].
/// - Equal rules digests: admitted. Lineage, packs and everything else in
///   `hello` are not compared (D10).
/// - Different digests: the first differing key is named
///   ([`Refusal::RulesDiffer`]); if every entry agrees, the digests cover
///   something the entries leave out ([`Refusal::RulesUnlisted`]).
pub fn admit(
    host_engine: &EngineId,
    host_rules: &dyn RulesIdentity,
    hello: &Hello,
    joiner_rules: &dyn RulesIdentity,
) -> Result<(), HandshakeError> {
    if !host_engine.matches(&hello.engine) {
        return Err(HandshakeError::Refused(Refusal::Engine {
            host: host_engine.clone(),
        }));
    }
    if hello.rules == host_rules.digest() {
        return Ok(());
    }
    let parts = rules_parts(&joiner_rules.entries()).map_err(HandshakeError::Protocol)?;
    let mut assembler = RulesAssembler::default();
    let mut theirs = None;
    for p in parts {
        theirs = assembler.push(p).map_err(HandshakeError::Protocol)?;
    }
    let theirs = theirs.ok_or(HandshakeError::Protocol(ProtoError::Invalid(
        "rules parts incomplete",
    )))?;
    let ours = wire_entries(&host_rules.entries()).map_err(HandshakeError::Protocol)?;
    Err(HandshakeError::Refused(match first_difference(&ours, &theirs) {
        Some(diff) => Refusal::RulesDiffer(diff),
        None => Refusal::RulesUnlisted,
    }))
}
