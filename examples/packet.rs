//! A condensed port of the C++ library's example.cpp: a game packet with a unified serialize
//! function, measured, written, sent "over the network", and read back defensively.

use serialize::{MeasureStream, ReadStream, Result, Serialize, Stream, WriteStream};

const MAX_PLAYERS: usize = 16;

#[derive(Default, Debug, PartialEq)]
struct Player {
    id: i32,
    x: f32,
    y: f32,
    health: i32,
    alive: bool,
}

impl Serialize for Player {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result {
        stream.serialize_int(&mut self.id, 0, 65535)?;
        stream.serialize_compressed_float(&mut self.x, -1000.0, 1000.0, 0.01)?;
        stream.serialize_compressed_float(&mut self.y, -1000.0, 1000.0, 0.01)?;
        stream.serialize_int(&mut self.health, 0, 100)?;
        stream.serialize_bool(&mut self.alive)?;
        Ok(())
    }
}

#[derive(Default, Debug, PartialEq)]
struct WorldStatePacket {
    sequence: u16,
    players: Vec<Player>,
    map_name: String,
}

impl Serialize for WorldStatePacket {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result {
        stream.serialize_u16(&mut self.sequence)?;

        // a serialized value that controls a loop must be validated before use: serialize_int
        // bounds it to [0,MAX_PLAYERS] and ? aborts on failure, so a malicious packet can
        // never drive this loop with garbage
        let mut num_players = self.players.len() as i32;
        stream.serialize_int(&mut num_players, 0, MAX_PLAYERS as i32)?;
        if S::IS_READING {
            self.players = (0..num_players).map(|_| Player::default()).collect();
        }
        for player in &mut self.players {
            player.serialize(stream)?;
        }

        stream.serialize_string(&mut self.map_name, 64)?;
        Ok(())
    }
}

fn main() -> Result {
    let mut packet = WorldStatePacket {
        sequence: 1000,
        players: vec![
            Player {
                id: 57,
                x: 100.0,
                y: 75.5,
                health: 100,
                alive: true,
            },
            Player {
                id: 12,
                x: -12.25,
                y: 3.0,
                health: 30,
                alive: true,
            },
            Player {
                id: 9,
                x: 0.0,
                y: 0.0,
                health: 0,
                alive: false,
            },
        ],
        map_name: "pressure".to_string(),
    };

    // measure how many bytes the packet needs (conservative: aligns count as 7 bits)
    let mut measure = MeasureStream::new();
    packet.serialize(&mut measure)?;
    println!("measured packet size: {} bytes", measure.bytes_processed());

    // write the packet
    let mut buffer = [0u8; 256]; // multiple of 8 bytes, comfortably above the measure
    let mut writer = WriteStream::new(&mut buffer);
    packet.serialize(&mut writer)?;
    writer.flush();
    let packet_bytes = writer.bytes_processed() as usize;
    println!("wrote packet: {packet_bytes} bytes");

    // ... the wire ...

    // read it back. the buffer extends past the packet data, so reads stay on the fast path
    let mut received = WorldStatePacket::default();
    let mut reader = ReadStream::new(&buffer, packet_bytes);
    received.serialize(&mut reader)?;

    assert_eq!(received.sequence, packet.sequence);
    assert_eq!(received.players.len(), packet.players.len());
    assert_eq!(received.map_name, packet.map_name);
    println!(
        "read packet back: sequence {} map {:?}",
        received.sequence, received.map_name
    );

    Ok(())
}
