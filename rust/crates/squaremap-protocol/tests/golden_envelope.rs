use prost::Message;
use squaremap_protocol::wire::{envelope, Envelope, Hello};

#[test]
fn encodes_v1_hello_golden_frame() {
    let envelope = Envelope {
        protocol_major: 1,
        protocol_minor: 0,
        session_id: hex::decode("00112233445566778899aabbccddeeff").unwrap(),
        sequence: 1,
        correlation_id: 0,
        payload: Some(envelope::Payload::Hello(Hello {
            plugin_version: "test".into(),
            bootstrap_token: b"token".to_vec(),
        })),
    };
    assert_eq!(
        envelope.encode_to_vec(),
        include_bytes!("../../../../testdata/bridge/v1/hello.bin")
    );
}
