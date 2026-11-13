//! NET-06: `Sections` (protocol tag 21) crosses the real transport: TCP, the
//! relay's stamp, the client's frame decoder and its bounds.

use std::thread;
use std::time::{Duration, Instant};

use sta_core::PlayerId;
use sta_net::lockstep::{Destination, Incoming, Transport};
use sta_net::proto::{Message, SectionReport, MAX_SECTIONS};
use sta_net::transport::{ClientOptions, NetTransport, Relay, RelayConfig};

fn recv(t: &mut NetTransport) -> Incoming {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(Some(m)) = t.poll() {
            return m;
        }
        assert!(Instant::now() < end, "no message arrived");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn section_reports_cross_the_relay_stamped_and_intact() {
    let relay = Relay::bind("127.0.0.1:0", RelayConfig::default()).unwrap();
    let o = ClientOptions::default();
    let mut host = NetTransport::create(relay.local_addr(), "", &o).unwrap();
    let code = host.control().room_code();
    let mut guest = NetTransport::join(relay.local_addr(), code, "", &o).unwrap();
    let report = SectionReport { tick: 90, digests: (1..=MAX_SECTIONS as u64).collect() };
    guest.send(Destination::All, &Message::Sections(report.clone())).unwrap();
    let got = recv(&mut host);
    assert_eq!(got.from, PlayerId(1));
    assert_eq!(got.msg, Message::Sections(report));
    // A report over the bound is refused before it is sent.
    let big = SectionReport { tick: 90, digests: vec![0; MAX_SECTIONS + 1] };
    assert!(guest.send(Destination::All, &Message::Sections(big)).is_err());
}
