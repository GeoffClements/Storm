// extern crate ../codec;
// use Storm::codec;
// use bytes::BytesMut;

// #[test]
// fn helo() {
//     let helo = codec::ClientMessage::Helo {
//         device_id: 0,
//         revision: 1,
//         mac: [2; 6],
//         uuid: [3; 16],
//         wlan_channel_list: 4,
//         bytes_received: 5,
//         capabilities: "test".to_owned(),
//     };

//     let helo = BytesMut::from(helo);

//     let msg = BytesMut::from(&[b'H', b'E', b'L', b'O', ]);

//     assert_eq!(helo, msg);
// }