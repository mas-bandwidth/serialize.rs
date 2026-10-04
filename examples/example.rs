//! A one-to-one port of the C++ library's example.cpp: a game packet with a unified serialize
//! function, measured, written, sent "over the network", and read back defensively.
//!
//! The C++ structs (`Vector`, `Quaternion`, `RigidBody`, `Address`, `PropertyValue`, the six
//! packet types and the packet factory) become Rust structs implementing the crate's
//! [`Serialize`] trait. The C program's `rand()` is replaced by a small deterministic PRNG
//! from the standard library, since the example takes no dependencies.

use serialize::{ReadStream, Serialize, Stream, WriteStream};

#[derive(Default, Clone, Copy, PartialEq, Debug)]
struct Vector {
    x: f32,
    y: f32,
    z: f32,
}

impl Serialize for Vector {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
        stream.serialize_f32(&mut self.x)?;
        stream.serialize_f32(&mut self.y)?;
        stream.serialize_f32(&mut self.z)?;
        Ok(())
    }
}

#[derive(Default, Clone, Copy, PartialEq, Debug)]
struct Quaternion {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
}

impl Serialize for Quaternion {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
        stream.serialize_f32(&mut self.x)?;
        stream.serialize_f32(&mut self.y)?;
        stream.serialize_f32(&mut self.z)?;
        stream.serialize_f32(&mut self.w)?;
        Ok(())
    }
}

#[derive(Default, Clone, Copy, PartialEq, Debug)]
struct RigidBody {
    position: Vector,
    orientation: Quaternion,
    linear_velocity: Vector,
    angular_velocity: Vector,
    at_rest: bool,
}

impl Serialize for RigidBody {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
        self.position.serialize(stream)?;
        self.orientation.serialize(stream)?;
        stream.serialize_bool(&mut self.at_rest)?;
        if !self.at_rest {
            self.linear_velocity.serialize(stream)?;
            self.angular_velocity.serialize(stream)?;
        } else if S::IS_READING {
            self.linear_velocity = Vector::default();
            self.angular_velocity = Vector::default();
        }
        Ok(())
    }
}

const ADDRESS_NONE: i32 = 0;
const ADDRESS_IPV4: i32 = 1;
const ADDRESS_IPV6: i32 = 2;
const NUM_ADDRESS_TYPES: i32 = 3;

#[allow(dead_code)]
#[allow(clippy::struct_field_names)]
#[derive(Default, Clone, Copy, PartialEq, Debug)]
struct Address {
    address_type: u8,
    ipv4: [u8; 4],
    ipv6: [u16; 8],
    port: u16,
}

#[allow(dead_code)]
impl Serialize for Address {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
        let mut address_type = i32::from(self.address_type);
        stream.serialize_int(&mut address_type, ADDRESS_NONE, NUM_ADDRESS_TYPES - 1)?;
        self.address_type = address_type as u8;

        if address_type == ADDRESS_IPV4 {
            for byte in &mut self.ipv4 {
                let mut value = u32::from(*byte);
                stream.serialize_bits(&mut value, 8)?;
                *byte = value as u8;
            }
        } else if address_type == ADDRESS_IPV6 {
            for word in &mut self.ipv6 {
                let mut value = u32::from(*word);
                stream.serialize_bits(&mut value, 16)?;
                *word = value as u16;
            }
        }

        let mut port = u32::from(self.port);
        stream.serialize_bits(&mut port, 16)?;
        self.port = port as u16;

        Ok(())
    }
}

const MAX_PROPERTIES: i32 = 1024;

const BOOL_PROPERTY: i32 = 0;
const BYTE_PROPERTY: i32 = 1;
const SHORT_PROPERTY: i32 = 2;
const INT_PROPERTY: i32 = 3;
const LONG_PROPERTY: i32 = 4;
const FLOAT_PROPERTY: i32 = 5;
const DOUBLE_PROPERTY: i32 = 6;
const NUM_PROPERTY_TYPES: i32 = 7;

#[derive(Clone, Copy, PartialEq, Debug)]
enum PropertyValue {
    Bool(bool),
    Byte(u8),
    Short(u16),
    Int(u32),
    Long(u64),
    Float(f32),
    Double(f64),
}

impl PropertyValue {
    fn property_type(&self) -> i32 {
        match self {
            PropertyValue::Bool(_) => BOOL_PROPERTY,
            PropertyValue::Byte(_) => BYTE_PROPERTY,
            PropertyValue::Short(_) => SHORT_PROPERTY,
            PropertyValue::Int(_) => INT_PROPERTY,
            PropertyValue::Long(_) => LONG_PROPERTY,
            PropertyValue::Float(_) => FLOAT_PROPERTY,
            PropertyValue::Double(_) => DOUBLE_PROPERTY,
        }
    }
}

impl Serialize for PropertyValue {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
        let mut property_type = if S::IS_WRITING {
            self.property_type()
        } else {
            0
        };
        stream.serialize_int(&mut property_type, 0, NUM_PROPERTY_TYPES - 1)?;

        if S::IS_READING {
            *self = match property_type {
                BOOL_PROPERTY => PropertyValue::Bool(false),
                BYTE_PROPERTY => PropertyValue::Byte(0),
                SHORT_PROPERTY => PropertyValue::Short(0),
                INT_PROPERTY => PropertyValue::Int(0),
                LONG_PROPERTY => PropertyValue::Long(0),
                FLOAT_PROPERTY => PropertyValue::Float(0.0),
                DOUBLE_PROPERTY => PropertyValue::Double(0.0),
                _ => unreachable!(),
            };
        }

        match self {
            PropertyValue::Bool(value) => stream.serialize_bool(value)?,
            PropertyValue::Byte(value) => {
                let mut raw = u32::from(*value);
                stream.serialize_bits(&mut raw, 8)?;
                *value = raw as u8;
            }
            PropertyValue::Short(value) => {
                let mut raw = u32::from(*value);
                stream.serialize_bits(&mut raw, 16)?;
                *value = raw as u16;
            }
            PropertyValue::Int(value) => stream.serialize_bits(value, 32)?,
            PropertyValue::Long(value) => stream.serialize_bits64(value, 64)?,
            PropertyValue::Float(value) => stream.serialize_f32(value)?,
            PropertyValue::Double(value) => stream.serialize_f64(value)?,
        }
        Ok(())
    }
}

const PACKET_TYPE_A: i32 = 0;
const PACKET_TYPE_B: i32 = 1;
const PACKET_TYPE_C: i32 = 2;
const PACKET_TYPE_D: i32 = 3;
const PACKET_TYPE_E: i32 = 4;
const PACKET_TYPE_F: i32 = 5;
const NUM_PACKET_TYPES: i32 = 6;

const PACKET_TYPE_STRING: [&str; 6] = [
    "packet type a",
    "packet type b",
    "packet type c",
    "packet type d",
    "packet type e",
    "packet type f",
];

const MAX_OBJECTS: i32 = 256;

#[derive(Default, Clone, PartialEq, Debug)]
struct PacketA {
    num_objects: i32,
    object: Vec<RigidBody>,
}

impl Serialize for PacketA {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
        stream.serialize_int(&mut self.num_objects, 0, MAX_OBJECTS)?;
        if S::IS_READING {
            self.object = vec![RigidBody::default(); self.num_objects as usize];
        }
        for body in &mut self.object {
            body.serialize(stream)?;
        }
        Ok(())
    }
}

#[derive(Default, Clone, Copy, PartialEq, Debug)]
struct PacketB {
    x: f32,
    y: f32,
    z: f32,
}

impl Serialize for PacketB {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
        stream.serialize_compressed_float(&mut self.x, -1.0, 1.0, 0.001)?;
        stream.serialize_compressed_float(&mut self.y, -1.0, 1.0, 0.001)?;
        stream.serialize_compressed_float(&mut self.z, -1.0, 1.0, 0.001)?;
        Ok(())
    }
}

#[derive(Default, Clone, PartialEq, Debug)]
struct PacketC {
    num_properties: i32,
    property_index: Vec<i32>,
    property_value: Vec<PropertyValue>,
}

impl Serialize for PacketC {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
        stream.serialize_int(&mut self.num_properties, 0, MAX_PROPERTIES)?;
        if S::IS_READING {
            self.property_index = vec![0; self.num_properties as usize];
            self.property_value = vec![PropertyValue::Bool(false); self.num_properties as usize];
        }
        // serialize_int_relative's domain is 0 to 2^31 - 1 for the previous value as well as
        // the current one, so the walk starts at the floor of the domain and the indices
        // start above it
        let mut last_property_index = 0;
        for i in 0..self.num_properties as usize {
            stream.serialize_int_relative(last_property_index, &mut self.property_index[i])?;
            last_property_index = self.property_index[i];
            self.property_value[i].serialize(stream)?;
        }
        Ok(())
    }
}

const MAX_CLIENTS: i32 = 8;
const MAX_PLAYER_NAME_LENGTH: usize = 64;
const PLAYER_DATA_BYTES: usize = 1024;

#[derive(Default, Clone, PartialEq, Debug)]
struct PacketD {
    client_index: i32,
    client_id: u64,
    player_name: String,
    has_player_data: bool,
    player_data: Vec<u8>,
}

impl Serialize for PacketD {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
        stream.serialize_int(&mut self.client_index, 0, MAX_CLIENTS - 1)?;
        stream.serialize_bits64(&mut self.client_id, 64)?;
        stream.serialize_string(&mut self.player_name, MAX_PLAYER_NAME_LENGTH)?;
        stream.serialize_bool(&mut self.has_player_data)?;
        if self.has_player_data {
            if S::IS_READING {
                self.player_data = vec![0u8; PLAYER_DATA_BYTES];
            }
            stream.serialize_bytes(&mut self.player_data)?;
        } else if S::IS_READING {
            self.player_data.clear();
        }
        Ok(())
    }
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Default, Clone, Copy, PartialEq, Debug)]
struct PacketE {
    i: bool,
    j: bool,
    k: bool,
    x: bool,
    y: bool,
    z: bool,
}

impl Serialize for PacketE {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
        stream.serialize_bool(&mut self.i)?;
        stream.serialize_bool(&mut self.j)?;
        stream.serialize_bool(&mut self.k)?;

        if self.i {
            stream.serialize_bool(&mut self.x)?;
            stream.serialize_bool(&mut self.y)?;
            stream.serialize_bool(&mut self.z)?;
        }

        stream.serialize_align()?;

        Ok(())
    }
}

#[derive(Default, Clone, Copy, PartialEq, Debug)]
struct PacketF {
    position_x: i64, // Q48.16 fixed point world position, in ±8192 whole units
    position_y: i64,
    position_z: i64,
    health: i32,      // Q16.16 fixed point health, in [0,100] whole units
    entity_id: u128,  // 128 bit globally unique entity id
    galactic_x: i128, // Q112.16 wide fixed point coordinate, in ±10^11 whole units
}

impl Serialize for PacketF {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
        stream.serialize_fixed(&mut self.position_x, 48, 16, -8192, 8192)?;
        stream.serialize_fixed(&mut self.position_y, 48, 16, -8192, 8192)?;
        stream.serialize_fixed(&mut self.position_z, 48, 16, -8192, 8192)?;
        stream.serialize_fixed(&mut self.health, 16, 16, 0, 100)?;
        stream.serialize_u128(&mut self.entity_id)?;
        stream.serialize_fixed(
            &mut self.galactic_x,
            112,
            16,
            -100_000_000_000,
            100_000_000_000,
        )?;
        Ok(())
    }
}

#[derive(Clone, PartialEq, Debug)]
enum Packet {
    A(PacketA),
    B(PacketB),
    C(PacketC),
    D(PacketD),
    E(PacketE),
    F(PacketF),
}

impl Default for Packet {
    fn default() -> Self {
        Packet::A(PacketA::default())
    }
}

impl Packet {
    fn packet_type(&self) -> i32 {
        match self {
            Packet::A(_) => PACKET_TYPE_A,
            Packet::B(_) => PACKET_TYPE_B,
            Packet::C(_) => PACKET_TYPE_C,
            Packet::D(_) => PACKET_TYPE_D,
            Packet::E(_) => PACKET_TYPE_E,
            Packet::F(_) => PACKET_TYPE_F,
        }
    }
}

impl Serialize for Packet {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
        let mut packet_type = if S::IS_WRITING { self.packet_type() } else { 0 };
        stream.serialize_int(&mut packet_type, 0, NUM_PACKET_TYPES - 1)?;

        if S::IS_READING {
            *self = match packet_type {
                PACKET_TYPE_A => Packet::A(PacketA::default()),
                PACKET_TYPE_B => Packet::B(PacketB::default()),
                PACKET_TYPE_C => Packet::C(PacketC::default()),
                PACKET_TYPE_D => Packet::D(PacketD::default()),
                PACKET_TYPE_E => Packet::E(PacketE::default()),
                PACKET_TYPE_F => Packet::F(PacketF::default()),
                _ => unreachable!(),
            };
        }

        match self {
            Packet::A(value) => value.serialize(stream)?,
            Packet::B(value) => value.serialize(stream)?,
            Packet::C(value) => value.serialize(stream)?,
            Packet::D(value) => value.serialize(stream)?,
            Packet::E(value) => value.serialize(stream)?,
            Packet::F(value) => value.serialize(stream)?,
        }

        let mut check_value = if S::IS_WRITING { 0x1234_5678 } else { 0 };
        stream.serialize_bits(&mut check_value, 32)?;
        if S::IS_READING && check_value != 0x1234_5678 {
            println!("error: serialize check failed");
            std::process::exit(1);
        }

        Ok(())
    }
}

/// The C program's `rand()`: a small deterministic PRNG, seeded from the wall clock to mirror
/// `srand(time(NULL))`.
struct Rand(u64);

impl Rand {
    fn seeded() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos() as u64);
        Rand(nanos | 1)
    }

    fn next(&mut self) -> i32 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as i32 & 0x7FFF_FFFF
    }
}

fn next_u64(rng: &mut Rand) -> u64 {
    (u64::from(rng.next() as u32) << 32) | u64::from(rng.next() as u32)
}

#[allow(clippy::too_many_lines)]
fn main() {
    println!("\nserialize example\n");

    let mut rng = Rand::seeded();

    for i in 0..10000 {
        let packet_type = rng.next() % NUM_PACKET_TYPES;

        let mut input = match packet_type {
            PACKET_TYPE_A => {
                let num_objects = rng.next() % (MAX_OBJECTS + 1);
                let mut object = Vec::with_capacity(num_objects as usize);
                for _ in 0..num_objects {
                    let mut body = RigidBody {
                        position: Vector {
                            x: ((rng.next() % 100) - 50) as f32,
                            y: ((rng.next() % 100) - 50) as f32,
                            z: ((rng.next() % 100) - 50) as f32,
                        },
                        orientation: Quaternion {
                            x: 1.0,
                            y: 0.0,
                            z: 0.0,
                            w: 0.0,
                        },
                        at_rest: (rng.next() % 2) == 0,
                        ..RigidBody::default()
                    };
                    if !body.at_rest {
                        body.linear_velocity = Vector {
                            x: 1.0,
                            y: 2.0,
                            z: 3.0,
                        };
                        body.angular_velocity = Vector {
                            x: 1.0,
                            y: 2.0,
                            z: 3.0,
                        };
                    }
                    object.push(body);
                }
                Packet::A(PacketA {
                    num_objects,
                    object,
                })
            }
            PACKET_TYPE_B => Packet::B(PacketB {
                x: ((rng.next() % 1_000_000) as f32) / 1_000_000.0 - 0.5,
                y: ((rng.next() % 1_000_000) as f32) / 1_000_000.0 - 0.5,
                z: ((rng.next() % 1_000_000) as f32) / 1_000_000.0 - 0.5,
            }),
            PACKET_TYPE_C => {
                let num_properties = rng.next() % (MAX_PROPERTIES + 1);

                // strictly greater than the walk's starting value of 0
                let mut property_index = Vec::with_capacity(num_properties as usize);
                let mut index = 1;
                for _ in 0..num_properties {
                    property_index.push(index);
                    index += 1 + rng.next() % 10;
                }

                let mut property_value = Vec::with_capacity(num_properties as usize);
                for _ in 0..num_properties {
                    let property_type = rng.next() % NUM_PROPERTY_TYPES;
                    let value = match property_type {
                        BOOL_PROPERTY => PropertyValue::Bool((rng.next() % 2) == 0),
                        BYTE_PROPERTY => PropertyValue::Byte((rng.next() % 256) as u8),
                        SHORT_PROPERTY => PropertyValue::Short((rng.next() % 65536) as u16),
                        INT_PROPERTY => PropertyValue::Int(rng.next() as u32),
                        LONG_PROPERTY => PropertyValue::Long(next_u64(&mut rng)),
                        FLOAT_PROPERTY => {
                            PropertyValue::Float((rng.next() % 10_000_000) as f32 / 1000.0)
                        }
                        DOUBLE_PROPERTY => PropertyValue::Double(
                            f64::from((rng.next() % 10_000_000) as u32) / 1000.0,
                        ),
                        _ => unreachable!(),
                    };
                    property_value.push(value);
                }

                Packet::C(PacketC {
                    num_properties,
                    property_index,
                    property_value,
                })
            }
            PACKET_TYPE_D => {
                let has_player_data = (rng.next() % 2) == 0;
                let player_data = if has_player_data {
                    (0..PLAYER_DATA_BYTES)
                        .map(|_| (rng.next() % 256) as u8)
                        .collect()
                } else {
                    Vec::new()
                };
                Packet::D(PacketD {
                    client_index: rng.next() % MAX_CLIENTS,
                    client_id: next_u64(&mut rng),
                    player_name: "Hingle McCringleberry".to_string(),
                    has_player_data,
                    player_data,
                })
            }
            PACKET_TYPE_E => {
                let mut packet = PacketE {
                    i: (rng.next() % 2) == 0,
                    j: (rng.next() % 2) == 0,
                    k: (rng.next() % 2) == 0,
                    ..PacketE::default()
                };
                if packet.i {
                    packet.x = (rng.next() % 2) == 0;
                    packet.y = (rng.next() % 2) == 0;
                    packet.z = (rng.next() % 2) == 0;
                }
                Packet::E(packet)
            }
            PACKET_TYPE_F => {
                // random raw Q48.16 values across the full ±8192 whole unit range, fraction
                // bits included
                let position_min: i64 = -8192 * 65536;
                let position_range: u64 = 16384 * 65536;
                let position_x = position_min + (next_u64(&mut rng) % (position_range + 1)) as i64;
                let position_y = position_min + (next_u64(&mut rng) % (position_range + 1)) as i64;
                let position_z = position_min + (next_u64(&mut rng) % (position_range + 1)) as i64;

                let health = (next_u64(&mut rng) % (100 * 65536 + 1)) as i32;

                let entity_id =
                    (u128::from(next_u64(&mut rng)) << 64) | u128::from(next_u64(&mut rng));

                let galactic_min: i128 = -100_000_000_000i128 * 65536;
                let galactic_range: u128 = 200_000_000_000u128 * 65536;
                let galactic_random =
                    (u128::from(next_u64(&mut rng)) << 64) | u128::from(next_u64(&mut rng));
                let galactic_x = galactic_min + (galactic_random % (galactic_range + 1)) as i128;

                Packet::F(PacketF {
                    position_x,
                    position_y,
                    position_z,
                    health,
                    entity_id,
                    galactic_x,
                })
            }
            _ => unreachable!(),
        };

        let mut buffer = vec![0u8; 100 * 1024];

        let mut write_stream = WriteStream::new(&mut buffer);
        let Ok(()) = input.serialize(&mut write_stream);
        write_stream.flush();

        let bytes_written = write_stream.bytes_processed() as usize;

        let mut output = Packet::default();
        let mut read_stream = ReadStream::new(&buffer, bytes_written);
        if output.serialize(&mut read_stream).is_err() {
            println!("error: serialize read failed");
            std::process::exit(1);
        }

        let bytes_read = read_stream.bytes_processed() as usize;

        if let (Packet::B(written), Packet::B(read)) = (&input, &output) {
            // compressed floats are quantized, so compare with a tolerance of the resolution
            // (contrast packet f: fixed point round trips are exact, so it takes the equality
            // path below with no carve out)
            if (written.x - read.x).abs() > 0.001
                || (written.y - read.y).abs() > 0.001
                || (written.z - read.z).abs() > 0.001
            {
                println!("error: packet read back does not match packet written");
                std::process::exit(1);
            }
        } else if input != output {
            println!("error: packet read back does not match packet written");
            std::process::exit(1);
        }

        println!(
            "{i}: {} - wrote {bytes_written} bytes, read {bytes_read} bytes",
            PACKET_TYPE_STRING[input.packet_type() as usize]
        );

        assert_eq!(bytes_written, bytes_read);
    }

    println!("\nSuccess!\n");
}
