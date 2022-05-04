use cosmrs::{
    bank::MsgSend,
    crypto::secp256k1,
    tx::{self, Fee, Msg, SignDoc, SignerInfo},
    AccountId, Coin
};

pub fn build_test_tx() -> Vec<u8> {
    // Generate sender private key.
    // In real world usage, this account would need to be funded before use.
    let sender_private_key = secp256k1::SigningKey::random();
    let sender_public_key = sender_private_key.public_key();
    let sender_account_id = sender_public_key.account_id("cosmos").unwrap();

    log::debug!("generated random cosmos address: {}", sender_account_id.to_string());

    // Parse recipient address from Bech32.
    let recipient_account_id =
        "cosmos19dyl0uyzes4k23lscla02n06fc22h4uqsdwq6z".parse::<AccountId>().unwrap();

    ///////////////////////////
    // Building transactions //
    ///////////////////////////

    // We'll be doing a simple send transaction.
    // First we'll create a "Coin" amount to be sent, in this case 1 million uatoms.
    let amount = Coin {
        amount: 1_000_000u64.into(),
        denom: "uatom".parse().unwrap(),
    };

    // Next we'll create a send message (from the "bank" module) for the coin
    // amount we created above.
    let msg_send = MsgSend {
        from_address: sender_account_id.clone(),
        to_address: recipient_account_id,
        amount: vec![amount.clone()],
    };

    // Transaction metadata: chain, account, sequence, gas, fee, timeout, and memo.
    let chain_id = "cosmoshub-4".parse().unwrap();
    let account_number = 1;
    let sequence_number = 0;
    let gas = 100_000;
    let timeout_height = 9001u16;
    let memo = "example memo";

    // Create transaction body from the MsgSend, memo, and timeout height.
    let tx_body = tx::Body::new(vec![msg_send.to_any().unwrap()], memo, timeout_height);

    // Create signer info from public key and sequence number.
    // This uses a standard "direct" signature from a single signer.
    let signer_info = SignerInfo::single_direct(Some(sender_public_key), sequence_number);

    // Compute auth info from signer info by associating a fee.
    let auth_info = signer_info.auth_info(Fee::from_amount_and_gas(amount, gas));

    //////////////////////////
    // Signing transactions //
    //////////////////////////

    // The "sign doc" contains a message to be signed.
    let sign_doc = SignDoc::new(&tx_body, &auth_info, &chain_id, account_number).unwrap();

    // Sign the "sign doc" with the sender's private key, producing a signed raw transaction.
    let tx_signed = sign_doc.sign(&sender_private_key).unwrap();
    
    tx_signed.to_bytes().unwrap()
}
