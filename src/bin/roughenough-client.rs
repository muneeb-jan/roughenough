// Copyright 2017-2021 int08h LLC

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

// for value_t_or_exit!()
#[macro_use]
extern crate clap;
extern crate log;
extern crate num_cpus;

use std::alloc::System;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Cursor, Write, self};
use std::iter::Iterator;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::thread;
use std::process::exit;
use std::thread::sleep;
use std::time::{self, SystemTime, UNIX_EPOCH, Instant};
use std::usize;
use std::sync::{Arc, Barrier};

use log::{LevelFilter, info};
use mio::Events;
use simple_logger::SimpleLogger;

use byteorder::{LittleEndian, ReadBytesExt};
use chrono::{Local, TimeZone};
use chrono::offset::Utc;
use clap::{App, Arg};
use data_encoding::{Encoding, HEXLOWER_PERMISSIVE, BASE64};
use ring::rand;
use ring::rand::SecureRandom;

use roughenough::{
    CERTIFICATE_CONTEXT, Error, RFC_REQUEST_FRAME_BYTES, roughenough_version, RtMessage, SIGNED_RESPONSE_CONTEXT,
    Tag, version,
};
use roughenough::merkle::MerkleTree;
use roughenough::sign::Verifier;
use roughenough::version::Version;

const HEX: Encoding = HEXLOWER_PERMISSIVE;

type Nonce = Vec<u8>;

fn create_nonce(ver: Version) -> Nonce {
    let rng = rand::SystemRandom::new();
    match ver {
        Version::Classic => {
            let mut nonce = [0u8; 64];
            rng.fill(&mut nonce).unwrap();
            nonce.to_vec()
        }
        Version::Rfc => {
            let mut nonce = [0u8; 32];
            rng.fill(&mut nonce).unwrap();
            nonce.to_vec()
        }
    }
}

fn make_request(ver: Version, nonce: &Nonce, text_dump: bool) -> Vec<u8> {
    //println!("[DEBUG_INFO] Entering MAKE REQUEST!");
    let mut msg = RtMessage::with_capacity(3);

    match ver {
        Version::Classic => {
            msg.add_field(Tag::NONC, nonce).unwrap();
            msg.add_field(Tag::PAD_CLASSIC, &[]).unwrap();

            let padding_needed = msg.calculate_padding_length();
            let padding: Vec<u8> = (0..padding_needed).map(|_| 0).collect();

            msg.clear();
            msg.add_field(Tag::NONC, nonce).unwrap();
            msg.add_field(Tag::PAD_CLASSIC, &padding).unwrap();

            if text_dump {
                eprintln!("Request = {}", msg);
            }

            msg.encode().unwrap()
        }
        Version::Rfc => {
            msg.add_field(Tag::PAD_RFC, &[]).unwrap();
            msg.add_field(Tag::VER, ver.wire_bytes()).unwrap();
            msg.add_field(Tag::NONC, nonce).unwrap();

            let padding_needed = msg.calculate_padding_length();
            let padding: Vec<u8> = (0..padding_needed).map(|_| 0).collect();

            msg.clear();
            msg.add_field(Tag::PAD_RFC, &padding).unwrap();
            msg.add_field(Tag::VER, ver.wire_bytes()).unwrap();
            msg.add_field(Tag::NONC, nonce).unwrap();

            if text_dump {
                eprintln!("Request = {}", msg);
            }

            //println!("[DEBUG_INFO] RETURNING MAKE REQUEST MSG!");
            msg.encode_framed().unwrap()
        }
    }
}

fn receive_response(ver: Version, buf: &[u8], buf_len: usize) -> RtMessage {
    match ver {
        Version::Classic => RtMessage::from_bytes(&buf[0..buf_len]).unwrap(),
        Version::Rfc => {
            verify_framing(&buf).unwrap();
            RtMessage::from_bytes(&buf[12..buf_len]).unwrap()
        }
    }
}

fn verify_framing(buf: &[u8]) -> Result<(), Error> {
    if &buf[0..8] != RFC_REQUEST_FRAME_BYTES {
        eprintln!("RFC response is missing framing header bytes");
        return Err(Error::InvalidResponse);
    }

    let mut cur = Cursor::new(&buf[8..12]);
    let reported_len = cur.read_u32::<LittleEndian>()?;

    if (reported_len as usize) > buf.len() - 12 {
        eprintln!("buflen = {}, reported_len = {}", buf.len(), reported_len);
        return Err(Error::MessageTooShort);
    }

    Ok(())
}

fn stress_test_forever(ver: Version, addr: &SocketAddr) -> () {
    // if !addr.ip().is_loopback() {
    //     panic!(
    //         "Cannot use non-loopback address {} for stress testing",
    //         addr.ip()
    //     );
    // }

    println!("Stress testing! for 10 seconds");

    let nonce = create_nonce(ver);
    let socket = UdpSocket::bind(if addr.is_ipv6() { "[::]:0" } else { "0.0.0.0:0" }).expect("Couldn't open UDP socket");
    let request = make_request(ver, &nonce, false);
    let start = Instant::now();
    println!("Start: {}", start.elapsed().as_millis());
    loop {
        socket.send_to(&request, addr).unwrap();
        let elapsed = start.elapsed();
        if elapsed.as_millis() > 10000 as u128{
            println!("End at {}", elapsed.as_millis());
            break;
    }
    }
    exit(0);
}

fn use_multithread_batching(num_threads: usize, version: Version, host: &str, port:u16, use_utc: bool) {
    println!("Multithreaded Batching starts!");
    let start_request = Instant::now();
    let time_format= "%b %d %Y %H:%M:%S.%f %Z";
    let addr = (host, port).to_socket_addrs().unwrap().next().unwrap();

    println!("Total cores: {}",  num_cpus::get());

    let mut handles = vec![];
    let barrier = Arc::new(Barrier::new(num_threads));

    for _ in 0..num_threads {
        let bar = Arc::clone(&barrier);
        let handle = thread::spawn(move || {
            let nonce = create_nonce(version);
            let socket = UdpSocket::bind(if addr.is_ipv6() { "[::]:0" } else { "0.0.0.0:0" }).expect("Couldn't open UDP socket");
            socket.set_nonblocking(true).unwrap();
            let request = make_request(version, &nonce, false);

            bar.wait(); //BARRIER HERE. THREADS WAIT TILL REQUESTS ARE FORMED AND THEN SEND AT SAME TIME.

            socket.send_to(&request, addr).unwrap();

            let mut buf = [0u8; 4096];
            let mut flag = 0;


            let (resp_len, _) = loop {
                match socket.recv_from(&mut buf) {
                    Ok(n) => break n,
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        if flag > 60 {
                            return;
                        }
                        thread::sleep(time::Duration::from_micros(3));
                        flag += 1;
                        continue;
                    },
                    Err(e) => panic!("encountered IO error: {}", e),
                }
            };

            let resp = receive_response(version, &buf, resp_len);

            let ParsedResponse {
                verified,
                midpoint,
                radius,
            } = ResponseHandler::new(version, None , resp.clone(), nonce.clone())
                .extract_time();


            let map = resp.into_hash_map();
            let index = map[&Tag::INDX]
                .as_slice()
                .read_u32::<LittleEndian>()
                .unwrap();
            let seconds = midpoint / 10_u64.pow(6);
            let nsecs = (midpoint - (seconds * 10_u64.pow(6))) * 10_u64.pow(3);
            let verify_str = if verified { "Yes" } else { "No" };

            let out = if use_utc {
                let ts = Utc.timestamp(seconds as i64, nsecs as u32);
                ts.format(time_format).to_string()
            } else {
                let ts = Local.timestamp(seconds as i64, nsecs as u32);
                ts.format(time_format).to_string()
            };
    
                //println!("[DEBUG_INFO] PRINT TIME AND OTHER INFO!");      
            info!(
            "Received time from server: midpoint={:?}, radius={:?}, verified={} flag={} (merkle_index={})",
            out, radius, verify_str, flag, index
            );
        
        });
        handles.push(handle);   
    }

    for handle in handles {
        handle.join().unwrap();
    }


        println!("Time taken to complete: {}", start_request.elapsed().as_micros());
    exit(0)
    
}

struct ResponseHandler {
    pub_key: Option<Vec<u8>>,
    msg: HashMap<Tag, Vec<u8>>,
    srep: HashMap<Tag, Vec<u8>>,
    cert: HashMap<Tag, Vec<u8>>,
    dele: HashMap<Tag, Vec<u8>>,
    nonce: Nonce,
    version: Version,
}

struct ParsedResponse {
    verified: bool,
    midpoint: u64,
    radius: u32,
}

impl ResponseHandler {
    pub fn new(
        version: Version,
        pub_key: Option<Vec<u8>>,
        response: RtMessage,
        nonce: Nonce,
    ) -> ResponseHandler {
        let msg = response.into_hash_map();
        let srep = RtMessage::from_bytes(&msg[&Tag::SREP])
            .unwrap()
            .into_hash_map();
        let cert = RtMessage::from_bytes(&msg[&Tag::CERT])
            .unwrap()
            .into_hash_map();
        let dele = RtMessage::from_bytes(&cert[&Tag::DELE])
            .unwrap()
            .into_hash_map();

        ResponseHandler {
            pub_key,
            msg,
            srep,
            cert,
            dele,
            nonce,
            version,
        }
    }

    pub fn extract_time(&self) -> ParsedResponse {
        let midpoint = self.srep[&Tag::MIDP]
            .as_slice()
            .read_u64::<LittleEndian>()
            .unwrap();
        let radius = self.srep[&Tag::RADI]
            .as_slice()
            .read_u32::<LittleEndian>()
            .unwrap();

        let verified = if self.pub_key.is_some() {
            self.validate_dele();
            self.validate_srep();
            self.validate_merkle();
            self.validate_midpoint(midpoint);
            true
        } else {
            false
        };

        ParsedResponse {
            verified,
            midpoint,
            radius,
        }
    }

    fn validate_dele(&self) {
        let mut full_cert = Vec::from(CERTIFICATE_CONTEXT.as_bytes());
        full_cert.extend(&self.cert[&Tag::DELE]);

        assert!(
            self.validate_sig(
                self.pub_key.as_ref().unwrap(),
                &self.cert[&Tag::SIG],
                &full_cert,
            ),
            "Invalid signature on DELE tag, response may not be authentic"
        );
    }

    fn validate_srep(&self) {
        let mut full_srep = Vec::from(SIGNED_RESPONSE_CONTEXT.as_bytes());
        full_srep.extend(&self.msg[&Tag::SREP]);

        assert!(
            self.validate_sig(&self.dele[&Tag::PUBK], &self.msg[&Tag::SIG], &full_srep),
            "Invalid signature on SREP tag, response may not be authentic"
        );
    }

    fn validate_merkle(&self) {
        let srep = RtMessage::from_bytes(&self.msg[&Tag::SREP])
            .unwrap()
            .into_hash_map();
        let index = self.msg[&Tag::INDX]
            .as_slice()
            .read_u32::<LittleEndian>()
            .unwrap();
        let paths = &self.msg[&Tag::PATH];

        let hash = match self.version {
            Version::Classic => MerkleTree::new_sha512(),
            Version::Rfc => MerkleTree::new_sha512_256(),
        }
        .root_from_paths(index as usize, &self.nonce, paths);

        assert_eq!(
            hash,
            srep[&Tag::ROOT],
            "Nonce is not present in the response's merkle tree"
        );
    }

    fn validate_midpoint(&self, midpoint: u64) {
        let mint = self.dele[&Tag::MINT]
            .as_slice()
            .read_u64::<LittleEndian>()
            .unwrap();
        let maxt = self.dele[&Tag::MAXT]
            .as_slice()
            .read_u64::<LittleEndian>()
            .unwrap();

        assert!(
            midpoint >= mint,
            "Response midpoint {} lies *before* delegation span ({}, {})",
            midpoint,
            mint,
            maxt
        );
        assert!(
            midpoint <= maxt,
            "Response midpoint {} lies *after* delegation span ({}, {})",
            midpoint,
            mint,
            maxt
        );
    }

    fn validate_sig(&self, public_key: &[u8], sig: &[u8], data: &[u8]) -> bool {
        let mut verifier = Verifier::new(public_key);
        verifier.update(data);
        verifier.verify(sig)
    }
}

fn main() {
    SimpleLogger::new()
        .with_level(LevelFilter::Info)
        .with_utc_timestamps()
        .init()
        .unwrap();
    
    //println!("[DEBUG_INFO] STARTING MAIN!");
    let matches = App::new("roughenough client")
        .version(roughenough_version().as_ref())
        .arg(Arg::with_name("host")
            .required(true)
            .help("The Roughtime server to connect to.")
            .takes_value(true))
        .arg(Arg::with_name("port")
            .required(true)
            .help("The Roughtime server port to connect to.")
            .takes_value(true))
        .arg(Arg::with_name("verbose")
            .short("v")
            .long("verbose")
            .help("Output additional details about the server's response."))
        .arg(Arg::with_name("dump")
            .short("d")
            .long("dump")
            .help("Pretty text dump of the exchanged Roughtime messages."))
        .arg(Arg::with_name("json")
            .short("j")
            .long("json")
            .help("Output the server's response in JSON format."))
        .arg(Arg::with_name("public-key")
            .short("k")
            .long("public-key")
            .takes_value(true)
            .help("The server public key used to validate responses. If unset, no validation will be performed."))
        .arg(Arg::with_name("multithread-batch")
            .short("m")
            .long("multithread-batch")
            .takes_value(true)
            .help("Use thread parallelization to create multiple requests. It will create m threads as specifided. (For efficiency m = num of cores)")
            .default_value("1")
        )
        .arg(Arg::with_name("time-format")
            .short("f")
            .long("time-format")
            .takes_value(true)
            .help("The strftime format string used to print the time received from the server.")
            .default_value("%b %d %Y %H:%M:%S.%f %Z")
        )
        .arg(Arg::with_name("num-requests")
            .short("n")
            .long("num-requests")
            .takes_value(true)
            .help("The number of requests to make to the server (each from a different source port). This is mainly useful for testing batch response handling.")
            .default_value("1")
        )
        .arg(Arg::with_name("stress")
            .short("s")
            .long("stress")
            .help("Stress test the server by sending the same request as fast as possible. Please only use this on your own server.")
        )
        .arg(Arg::with_name("output-requests")
            .short("o")
            .long("output-requests")
            .takes_value(true)
            .help("Writes all requests to the specified file, in addition to sending them to the server. Useful for generating fuzzer inputs.")
        )
        .arg(Arg::with_name("output-responses")
            .short("O")
            .long("output-responses")
            .takes_value(true)
            .help("Writes all server responses to the specified file, in addition to processing them. Useful for generating fuzzer inputs.")
        )
        .arg(Arg::with_name("protocol")
            .short("p")
            .long("protocol")
            .takes_value(true)
            .help("Roughtime protocol version to use (0 = classic, 1 = rfc)")
            .default_value("0")
        )
        .arg(Arg::with_name("zulu")
            .short("z")
            .long("zulu")
            .help("Display time in UTC (default is local time zone)")
        )
        .get_matches();

    
    
    let host = matches.value_of("host").unwrap();
    let port = value_t_or_exit!(matches.value_of("port"), u16);
    let verbose = matches.is_present("verbose");
    let text_dump = matches.is_present("dump");
    let json = matches.is_present("json");
    let multithread_batch = value_t_or_exit!(matches.value_of("multithread-batch"), u16) as usize;
    let num_requests = value_t_or_exit!(matches.value_of("num-requests"), u16) as usize;
    let time_format = matches.value_of("time-format").unwrap();
    let stress = matches.is_present("stress");
    let pub_key = matches.value_of("public-key").map(|pkey| {
        HEX.decode(pkey.as_ref())
            .or_else(|_| BASE64.decode(pkey.as_ref()))
            .expect("Error parsing public key!")
    });
    let output_requests = matches.value_of("output-requests");
    let output_responses = matches.value_of("output-responses");
    let protocol = value_t_or_exit!(matches.value_of("protocol"), u8);
    let use_utc = matches.is_present("zulu");
    let start_request = Instant::now();
    if verbose {
        info!("Requesting time from: {:?}:{:?}", host, port);
    }

    let version = match protocol {
        0 => Version::Classic,
        1 => Version::Rfc,
        _ => panic!("Invalid protocol '{}'; valid values are 0 or 1", protocol),
    };

    let addr = (host, port).to_socket_addrs().unwrap().next().unwrap();

    if stress {
        stress_test_forever(version, &addr)
    }

    if multithread_batch > 1 {
        use_multithread_batching(multithread_batch, version, host, port, use_utc)
    }


    // let start = Instant::now();
    // loop{
        //println!("[DEBUG_INFO] START ADDING REQUESTS!");

    let mut requests = Vec::with_capacity(num_requests);
    let mut file_for_requests =
        output_requests.map(|o| File::create(o).expect("Failed to create file!"));
    let mut file_for_responses =
        output_responses.map(|o| File::create(o).expect("Failed to create file!"));


    for _ in 0..num_requests {
        let nonce = create_nonce(version);
        let socket = UdpSocket::bind(if addr.is_ipv6() { "[::]:0" } else { "0.0.0.0:0" }).expect("Couldn't open UDP socket");
            socket.set_nonblocking(true).unwrap();
        let request = make_request(version, &nonce, text_dump);

        if let Some(f) = file_for_requests.as_mut() {
            f.write_all(&request).expect("Failed to write to file!")
        }

        requests.push((nonce, request, socket));
    }

        //println!("[DEBUG_INFO] START SENDING REQUESTS!");
    for &mut (_, ref request, ref mut socket) in &mut requests {
        socket.send_to(request, addr).unwrap();
    }

        //println!("[DEBUG_INFO] START COLLECTING RESPONSES LOOP!");    
    'outer: for (nonce, _, socket) in requests {
            //println!("[DEBUG_INFO] ENTER COLLECTING RESPONSES LOOP!");
        let mut buf = [0u8; 4096];
        let mut flag = false;

        let (resp_len, _) = loop {
            match socket.recv_from(&mut buf) {
                Ok(n) => break n,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if flag {
                        flag = false;
                        continue 'outer;
                    }
                    sleep(time::Duration::from_micros(100));
                    flag = true;
                    continue;
                },
                Err(e) => panic!("encountered IO error: {}", e),
            }
        };
            // let resp = socket.recv_from(&mut buf);
            // let resp_len = match resp {
            //     Ok((sz, _saddr)) => sz,
            //     Err(_error) => continue
            // };
            // ^^ OLD way of doing non-blocking. It used to skip a lot of sockets without waiting for any....
            // NON BLOCKING LOOP: Should actually be implemented inside a loop with a wait mechanism. For more details, refer to ...
            // https://doc.rust-lang.org/std/net/struct.UdpSocket.html#method.set_nonblocking
            // I want control to return to main loop.


        if let Some(f) = file_for_responses.as_mut() {
            f.write_all(&buf[0..resp_len])
                .expect("Failed to write to file!")
        }

            //println!("[DEBUG_INFO] EXTRACT MESSAGE FROM RESPONSES!");     
        let resp = receive_response(version, &buf, resp_len);

        if text_dump {
            eprintln!("Response = {}", resp);
        }

            //println!("[DEBUG_INFO] PARSE RECEIVED MESSAGE!");     
        let ParsedResponse {
            verified,
            midpoint,
            radius,
        } = ResponseHandler::new(version, pub_key.clone(), resp.clone(), nonce.clone())
            .extract_time();

        let map = resp.into_hash_map();
        let index = map[&Tag::INDX]
            .as_slice()
            .read_u32::<LittleEndian>()
            .unwrap();

            //println!("[DEBUG_INFO] CALCULATE TIME FROM SREP IN MESSAGE!");  
        let seconds = midpoint / 10_u64.pow(6);
        let nsecs = (midpoint - (seconds * 10_u64.pow(6))) * 10_u64.pow(3);
        let verify_str = if verified { "Yes" } else { "No" };

        let out = if use_utc {
            let ts = Utc.timestamp(seconds as i64, nsecs as u32);
            ts.format(time_format).to_string()
        } else {
            let ts = Local.timestamp(seconds as i64, nsecs as u32);
            ts.format(time_format).to_string()
        };

            //println!("[DEBUG_INFO] PRINT TIME AND OTHER INFO!");      
        if verbose {

                info!(
                "Received time from server: midpoint={:?}, radius={:?}, verified={} flag={} (merkle_index={})",
                out, radius, verify_str, flag, index
            );

        }

        if json {
            println!(
                r#"{{ "midpoint": {:?}, "radius": {:?}, "verified": {}, "merkle_index": {} }}"#,
                out, radius, verified, index
            );
        } else {
            println!("{}", out);
        }
    }
        println!("Time taken to complete: {}", start_request.elapsed().as_micros());
        //println!("[DEBUG_INFO] MAIN ENDS!");   
    //     let elapsed = start.elapsed();
    //     if elapsed.as_millis() > 10000 as u128{
    //         println!("End at {}", elapsed.as_millis());
    //         break;
    //     }
    // }
}
