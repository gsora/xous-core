use std::io::ErrorKind;

use bincode::{Decode, Encode};
use pddb::{Pddb, PddbKey};

const DICT_NAME: &'static str = "irc";
const USER_NETWORKS_KEY: &'static str = "user_networks";
const CANARY_KEY: &'static str = "canary";

#[derive(Encode, Decode, PartialEq, Default, Debug, Clone)]
pub struct Network {
    pub name: std::string::String,
    pub channel: std::string::String,
    pub server: std::string::String,
    pub nickname: std::string::String,
}

fn create_dict(pddb: &mut Pddb) {
    match pddb.get(DICT_NAME, CANARY_KEY, None, true, true, None, Some(|| {})) {
        Err(error) => panic!("cannot create canary key, {}", error),
        Ok(_) => (),
    };
}

pub fn get_networks(pddb: &mut Pddb) -> Result<Vec<Network>, Box<dyn std::error::Error>> {
    let keys_list = match pddb.list_keys(DICT_NAME, None) {
        Err(ref error) if error.kind() == ErrorKind::NotFound => {
            log::debug!("irc dict doesn't exist in basis, creating canary");

            create_dict(pddb);

            Vec::<std::string::String>::new()
        }
        Err(error) => return Err(Box::new(error)),
        Ok(list) => list,
    };

    // ignore "canary"
    let keys_list = keys_list.into_iter()
        .filter(|element| {
            is_valid_network_key(element)    
        })
        .collect::<Vec<_>>();

    let mut ret = vec![];
    for key in keys_list {
        let network = get_network_lowlevel(key, pddb)?;
        ret.push(network);
    }

    Ok(ret)
}

pub fn store_network(network: Network, pddb: &mut Pddb) -> Result<(), Box<dyn std::error::Error>> {
    let mut new_network: PddbKey = match pddb.get(
        DICT_NAME,
        &network_pddb_key(network.name.clone()),
        None,
        true,
        true,
        None,
        Some(|| {}),
    ) {
        Err(error) => return Err(Box::new(error)),
        Ok(data) => data,
    };

    if let Err(error) = bincode::encode_into_std_write(
        network,
        &mut new_network,
        bincode::config::standard().with_big_endian(),
    ) {
        Err(Box::new(error))
    } else {
        Ok(())
    }
}

pub fn get_network(name: String, pddb: &mut Pddb) -> Result<Network, Box<dyn std::error::Error>> {
    get_network_lowlevel(network_pddb_key(name.clone()), pddb)
}

fn get_network_lowlevel(
    key: String,
    pddb: &mut Pddb,
) -> Result<Network, Box<dyn std::error::Error>> {
    let mut network: PddbKey = match pddb.get(DICT_NAME, &key, None, true, false, None, Some(|| {}))
    {
        Err(error) => return Err(Box::new(error)),
        Ok(data) => data,
    };

    match bincode::decode_from_std_read(&mut network, bincode::config::standard().with_big_endian())
    {
        Ok(networks) => return Ok(networks),
        Err(error) => return Err(Box::new(error)),
    };
}

fn network_pddb_key(name: std::string::String) -> std::string::String {
    let mut key = USER_NETWORKS_KEY.to_string();
    key.push_str(&name);
    key
}

fn is_valid_network_key(key: &std::string::String) -> bool {
    key.starts_with(USER_NETWORKS_KEY)
}
