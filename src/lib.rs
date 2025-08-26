use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use bitcoincore_rpc::bitcoin::Network::Regtest;
use bitcoincore_rpc::bitcoin::{Address, Amount, Network, Txid};
use bitcoincore_rpc::bitcoincore_rpc_json::{AddressType, GetTransactionResult};
use bitcoincore_rpc::json::LoadWalletResult;
use bitcoincore_rpc::{Auth, Client, Error as RpcError, RpcApi};
use dotenv as env;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::HashMap;
use std::ops::Deref;
use std::str::FromStr;
use actix_cors::Cors;
use actix_web::http::header;
use dashmap::DashMap;
// const INITIAL_MINING_BLOCKS: u64 = 101;
// const REQUIRED_MINER_BALANCE: f64 = 20.0;
// const TRANSFER_AMOUNT: u64 = 20;

// Request/Response structs for API
#[derive(Deserialize)]
struct CreateWalletRequest {
    name: String,
}

#[derive(Deserialize)]
struct CreateWalletAddress {
    wallet_name: String,
    name: String,
}

#[derive(Deserialize)]
struct MineBlockRequest {
    wallet_name: String,
    address: String,
    blocks: u64,
}

#[derive(Deserialize)]
struct SendBitcoinRequest {
    from_wallet: String,
    to_address: String,
    amount: u64,
    message: Option<String>,
}

// AppState to hold shared configuration
struct AppState {
    config: Config,
    clients: DashMap<String, Client>,
}

#[derive(Debug)]
struct Config {
    rpc_url: String,
    rpc_user: String,
    rpc_password: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct TransactionDetails {
    txid: String,
    miner_input_address: String,
    miner_input_amount: f64,
    trader_output_address: String,
    trader_output_amount: f64,
    miner_change_address: String,
    miner_change_amount: f64,
    fee: f64,
    block_height: u64,
    confirmation_block_hash: String,
}

impl Config {
    fn from_env() -> Result<Self, RpcError> {
        Ok(Self {
            rpc_user: env::var("user").map_err(|_| {
                RpcError::ReturnedError("cannot load username from env file".into())
            })?,
            rpc_password: env::var("password").map_err(|_| {
                RpcError::ReturnedError("cannot load password from env file".into())
            })?,
            rpc_url: env::var("rpc_url")
                .map_err(|_| RpcError::ReturnedError("cannot load rpc-url from env file".into()))?,
        })
    }

    fn create_client(&self, wallet: &str) -> Result<Client, RpcError> {
        Client::new(
            format!("{}/wallet/{}", self.rpc_url, wallet).as_str(),
            Auth::UserPass(self.rpc_user.clone(), self.rpc_password.clone()),
        )
    }
}

// API handlers
async fn create_wallet(
    data: web::Data<AppState>,
    req: web::Json<CreateWalletRequest>,
) -> impl Responder {
    let config = &data.config;
    let client = match config.create_client(&req.name) {
        Ok(client) => client,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    match get_wallet(&client, &req.name) {
        Ok(result) => {
            let clients = &data.clients;
            clients.insert(req.name.clone(), client);
            HttpResponse::Ok().json(result)
        }
        Err(e) => HttpResponse::InternalServerError().body(e.to_string()),
    }
}

// Generate spendable balances in the Miner wallet
async fn create_address(
    data: web::Data<AppState>,
    req: web::Json<CreateWalletAddress>,
) -> impl Responder {
    let clients = &data.clients;
    if let Some(client) = clients.get(&req.wallet_name) {
        let address =
            match client.get_new_address(Some(req.name.as_str()), Some(AddressType::Bech32)) {
                Ok(addr) => match addr.require_network(Network::Regtest) {
                    Ok(addr) => addr,
                    Err(e) => {
                        return HttpResponse::BadRequest()
                            .body(format!("Address generated with error: {e}"));
                    }
                },
                Err(e) => {
                    return HttpResponse::BadRequest()
                        .body(format!("Failed to generate a new address: {e}"))
                }
            };
        HttpResponse::Ok().json(address)
    } else {
        HttpResponse::NotFound().body("No such wallet")
    }
}

async fn get_balance(
    data: web::Data<AppState>,
    walletid: web::Path<String>,
) -> impl Responder {
    let clients = &data.clients;
    println!("Getting balance for: {:?}", &walletid.deref());
    if let Some(client) = clients.get(walletid.as_str()) {
        match client.get_wallet_info() {
            Ok(info) => HttpResponse::Ok().json(info.balance.to_sat()),
            Err(e) => HttpResponse::InternalServerError().body(e.to_string()),
        }
    } else { HttpResponse::NotFound().body("No such wallet") }
}

async fn mine_blocks(
    data: web::Data<AppState>,
    req: web::Json<MineBlockRequest>,
) -> impl Responder {
    let clients = &data.clients;
    if let Some(client) = clients.get(&req.wallet_name) {
        let address = match Address::from_str(&req.address) {
            Ok(addr) => match addr.require_network(Network::Regtest) {
                Ok(addr) => addr,
                Err(e) => {
                    return HttpResponse::BadRequest().body(format!("Invalid network: {}", e))
                }
            },
            Err(e) => return HttpResponse::BadRequest().body(format!("Invalid address: {}", e)),
        };

        match client.generate_to_address(req.blocks, &address) {
            Ok(block_hashes) => HttpResponse::Ok().json(block_hashes),
            Err(e) => HttpResponse::InternalServerError().body(e.to_string()),
        }
    } else {
        HttpResponse::NotFound().body("Wallet not found")
    }
}

async fn send_bitcoin(
    data: web::Data<AppState>,
    req: web::Json<SendBitcoinRequest>,
) -> impl Responder {
    let clients = &data.clients;
    if let Some(client) = clients.get(&req.from_wallet) {
        let to_address = match Address::from_str(&req.to_address) {
            Ok(addr) => match addr.require_network(Regtest) {
                Ok(addr) => addr,
                Err(e) => {
                    return HttpResponse::BadRequest().body(format!("Invalid network: {}", e))
                }
            },
            Err(e) => return HttpResponse::BadRequest().body(format!("Invalid address: {}", e)),
        };

        let amount = Amount::from_sat(req.amount);
        match client.send_to_address(
            &to_address,
            amount,
            req.message.as_deref(),
            None,
            None,
            None,
            None,
            None,
        ) {
            Ok(txid) => HttpResponse::Ok().json(txid.to_string()),
            Err(e) => HttpResponse::InternalServerError().body(e.to_string()),
        }
    } else {
        HttpResponse::NotFound().body("Wallet not found")
    }
}

struct GetTransactionResultWrapper(GetTransactionResult);

impl Serialize for GetTransactionResultWrapper {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut tx = serializer.serialize_struct("Transaction", 10)?;
        tx.serialize_field("txid", &self.0.info.txid.to_string())?;
        tx.serialize_field("blockhash", &self.0.info.blockhash)?;
        tx.serialize_field("blockindex", &self.0.info.blockindex)?;
        tx.serialize_field("blockheight", &self.0.info.blockheight.expect("REASON"))?;
        tx.serialize_field("bip125_replaceable", &self.0.info.bip125_replaceable)?;
        tx.serialize_field("blocktime", &self.0.info.blocktime)?;
        tx.serialize_field("confirmations", &self.0.info.confirmations)?;
        tx.serialize_field("time", &self.0.info.time)?;
        tx.serialize_field("timereceived", &self.0.info.timereceived)?;
        tx.serialize_field("wallet_conflicts", &self.0.info.wallet_conflicts)?;
        tx.serialize_field("amount", &self.0.amount.to_btc())?;
        // todo: fix below
        for detail in self.0.details.iter() {
            tx.serialize_field("address", &detail.address)?;
            tx.serialize_field("vout", &detail.vout)?;
            tx.serialize_field("category", &detail.category)?;
            tx.serialize_field("label", &detail.label)?;
        }
        if let Some(fee) = &self.0.fee {
            tx.serialize_field("fee", &fee.to_btc())?;
        }

        let encoded_tx = hex::encode(&self.0.hex);
        tx.serialize_field("hex", &encoded_tx)?;

        tx.end()
    }
}

async fn get_transaction(data: web::Data<AppState>, path: web::Path<(String, String)>) -> impl Responder {
    let (walletid, txid) = path.into_inner();
    if let Some(client) = data.clients.get(walletid.as_str()) {
        let txid = match Txid::from_str(&txid) {
            Ok(id) => id,
            Err(e) => {
                return HttpResponse::BadRequest().body(format!("Invalid transaction ID: {}", e))
            }
        };

        match client.get_transaction(&txid, None) {
            Ok(tx) => HttpResponse::Ok().json(GetTransactionResultWrapper(tx)),
            Err(e) => HttpResponse::NotFound().body(e.to_string()),
        }
    } else {
        HttpResponse::ServiceUnavailable().body("No active clients")
    }
}

async fn get_mempool_entry(data: web::Data<AppState>, path: web::Path<(String, String)>) -> impl Responder {
    let (walletid, txid) = path.into_inner();
    if let Some(client) = data.clients.get(walletid.as_str()) {
        let txid = match Txid::from_str(&txid) {
            Ok(id) => id,
            Err(e) => {
                return HttpResponse::BadRequest().body(format!("Invalid transaction ID: {}", e))
            }
        };

        match client.get_mempool_entry(&txid) {
            Ok(entry) => HttpResponse::Ok().json(entry),
            Err(e) => HttpResponse::NotFound().body(e.to_string()),
        }
    } else {
        HttpResponse::ServiceUnavailable().body("No active clients")
    }
}

pub async fn run_server() -> std::io::Result<()> {
    env_logger::init();

    let config = Config::from_env().expect("Failed to load config");
    let app_state = web::Data::new(AppState {
        config,
        clients: DashMap::new(),
    });

    println!("Starting server at http://127.0.0.1:8021");

    HttpServer::new(move || {
        let cors = Cors::default()
            .allowed_origin("http://localhost:8080")
            .allowed_methods(vec!["GET", "POST"])
            .allowed_headers(vec![header::AUTHORIZATION, header::ACCEPT, header::CONTENT_TYPE])
            .max_age(3600);
        App::new()
            .wrap(cors)
            .app_data(app_state.clone())
            .route("/wallet", web::post().to(create_wallet))
            .route("/address", web::post().to(create_address))
            .route("/mine", web::post().to(mine_blocks))
            .route("/wallet/{walletid}/balance", web::get().to(get_balance))
            .route("/send", web::post().to(send_bitcoin))
            .route("/tx/{walletid}/{txid}", web::get().to(get_transaction))
            .route("/mempool/{walletid}/{txid}", web::get().to(get_mempool_entry))
    })
    .bind("127.0.0.1:8021")?
    .run()
    .await
}

fn get_wallet(rpc: &Client, wallet_name: &str) -> bitcoincore_rpc::Result<LoadWalletResult> {
    // Check if wallet exists
    let wallets = rpc.list_wallets()?;
    let wallet_exists = wallets.iter().any(|wallet| wallet == wallet_name);

    if wallet_exists {
        // Try loading the wallet
        match rpc.load_wallet(wallet_name) {
            Ok(result) => Ok(result),
            Err(e) => {
                // If error is "already loaded" (code -4), unload and retry
                if e.to_string().contains("code: -4") {
                    rpc.unload_wallet(Some(wallet_name))?;
                    rpc.load_wallet(wallet_name)
                } else {
                    Err(e)
                }
            }
        }
    } else {
        // Try creating a new wallet
        rpc.create_wallet(wallet_name, None, None, None, None)
            .map_err(|e| {
                if e.to_string().contains("code: -4") {
                    RpcError::ReturnedError("Wallet already exists but was not listed".into())
                } else {
                    e
                }
            })
    }
}
