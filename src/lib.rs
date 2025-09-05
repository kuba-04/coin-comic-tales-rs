use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use bitcoincore_rpc::bitcoin::Network::Regtest;
use bitcoincore_rpc::bitcoin::{Address, Amount, Network, Txid};
use bitcoincore_rpc::bitcoincore_rpc_json::{AddressType, GetTransactionResult};
use bitcoincore_rpc::json::LoadWalletResult;
use bitcoincore_rpc::{Auth, Client, Error as RpcError, RpcApi};
use dotenv as env;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};
use std::str::FromStr;
use actix_cors::Cors;
use actix_web::http::header;
use actix_web::middleware::Logger as ActixLogger;
use dashmap::DashMap;
use log::{debug, error, info, warn};

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
    server_url: String,
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
            server_url: env::var("server_url")
                .map_err(|_| RpcError::ReturnedError("cannot load server-url from env file".into()))?,
        })
    }

    fn create_client(&self, wallet: &str) -> Result<Client, RpcError> {
        let url = format!("{}/wallet/{}", self.rpc_url, wallet);
        debug!("Creating RPC client for wallet '{}' at {}", wallet, url);
        Client::new(
            url.as_str(),
            Auth::UserPass(self.rpc_user.clone(), self.rpc_password.clone()),
        )
    }
}

// API handlers
async fn create_wallet(
    data: web::Data<AppState>,
    req: web::Json<CreateWalletRequest>,
) -> impl Responder {
    info!("POST /wallet - creating or loading wallet '{}'", req.name);
    let config = &data.config;
    let client = match config.create_client(&req.name) {
        Ok(client) => client,
        Err(e) => {
            error!("Failed to create RPC client for wallet '{}': {}", req.name, e);
            return HttpResponse::InternalServerError().body(e.to_string());
        }
    };

    match get_wallet(&client, &req.name) {
        Ok(result) => {
            info!("Wallet '{}' is ready (loaded or created)", req.name);
            let clients = &data.clients;
            clients.insert(req.name.clone(), client);
            HttpResponse::Ok().json(result)
        }
        Err(e) => {
            error!("Failed to load/create wallet '{}': {}", req.name, e);
            HttpResponse::InternalServerError().body(e.to_string())
        }
    }
}

// Generate spendable balances in the Miner wallet
async fn create_address(
    data: web::Data<AppState>,
    req: web::Json<CreateWalletAddress>,
) -> impl Responder {
    info!(
        "POST /address - wallet='{}', label='{}'",
        req.wallet_name, req.name
    );
    let clients = &data.clients;
    if let Some(client) = clients.get(&req.wallet_name) {
        let address =
            match client.get_new_address(Some(req.name.as_str()), Some(AddressType::Bech32)) {
                Ok(addr) => match addr.require_network(Network::Regtest) {
                    Ok(addr) => addr,
                    Err(e) => {
                        error!("Generated address wrong network for wallet '{}': {}", req.wallet_name, e);
                        return HttpResponse::BadRequest()
                            .body(format!("Address generated with error: {e}"));
                    }
                },
                Err(e) => {
                    error!("Failed to get new address for wallet '{}': {}", req.wallet_name, e);
                    return HttpResponse::BadRequest()
                        .body(format!("Failed to generate a new address: {e}"))
                }
            };
        info!("New address generated for wallet '{}': {}", req.wallet_name, address);
        HttpResponse::Ok().json(address)
    } else {
        warn!("POST /address - wallet '{}' not found", req.wallet_name);
        HttpResponse::NotFound().body("No such wallet")
    }
}

async fn get_balance(
    data: web::Data<AppState>,
    walletid: web::Path<String>,
) -> impl Responder {
    info!("GET /wallet/{}/balance", walletid);
    let clients = &data.clients;
    if let Some(client) = clients.get(walletid.as_str()) {
        match client.get_wallet_info() {
            Ok(info) => {
                debug!("Wallet '{}' balance: {} sat", walletid, info.balance.to_sat());
                HttpResponse::Ok().json(info.balance.to_sat())
            }
            Err(e) => {
                error!("Failed to get balance for wallet '{}': {}", walletid, e);
                HttpResponse::InternalServerError().body(e.to_string())
            }
        }
    } else { 
        warn!("GET /wallet/{}/balance - wallet not found", walletid);
        HttpResponse::NotFound().body("No such wallet") 
    }
}

async fn mine_blocks(
    data: web::Data<AppState>,
    req: web::Json<MineBlockRequest>,
) -> impl Responder {
    info!(
        "POST /mine - wallet='{}', address='{}', blocks={}",
        req.wallet_name, req.address, req.blocks
    );
    let clients = &data.clients;
    if let Some(client) = clients.get(&req.wallet_name) {
        let address = match Address::from_str(&req.address) {
            Ok(addr) => match addr.require_network(Network::Regtest) {
                Ok(addr) => addr,
                Err(e) => {
                    error!("Mine request wrong network for wallet '{}': {}", req.wallet_name, e);
                    return HttpResponse::BadRequest().body(format!("Invalid network: {}", e))
                }
            },
            Err(e) => {
                error!("Mine request invalid address for wallet '{}': {}", req.wallet_name, e);
                return HttpResponse::BadRequest().body(format!("Invalid address: {}", e))
            },
        };

        match client.generate_to_address(req.blocks, &address) {
            Ok(block_hashes) => {
                info!("Mined {} blocks to {} for wallet '{}'", req.blocks, req.address, req.wallet_name);
                HttpResponse::Ok().json(block_hashes)
            }
            Err(e) => {
                error!("Failed to mine blocks for wallet '{}': {}", req.wallet_name, e);
                HttpResponse::InternalServerError().body(e.to_string())
            }
        }
    } else {
        warn!("POST /mine - wallet '{}' not found", req.wallet_name);
        HttpResponse::NotFound().body("Wallet not found")
    }
}

async fn send_bitcoin(
    data: web::Data<AppState>,
    req: web::Json<SendBitcoinRequest>,
) -> impl Responder {
    info!(
        "POST /send - from='{}', to='{}', amount_sat={}, has_message={}",
        req.from_wallet,
        req.to_address,
        req.amount,
        req.message.as_ref().map(|m| !m.is_empty()).unwrap_or(false)
    );
    let clients = &data.clients;
    if let Some(client) = clients.get(&req.from_wallet) {
        let to_address = match Address::from_str(&req.to_address) {
            Ok(addr) => match addr.require_network(Regtest) {
                Ok(addr) => addr,
                Err(e) => {
                    error!("Send invalid network from wallet '{}': {}", req.from_wallet, e);
                    return HttpResponse::BadRequest().body(format!("Invalid network: {}", e))
                }
            },
            Err(e) => {
                error!("Send invalid address for wallet '{}': {}", req.from_wallet, e);
                return HttpResponse::BadRequest().body(format!("Invalid address: {}", e))
            },
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
            Ok(txid) => {
                info!("Sent {} sat from '{}' to '{}' txid={}", req.amount, req.from_wallet, req.to_address, txid);
                HttpResponse::Ok().json(txid.to_string())
            }
            Err(e) => {
                error!("Failed to send from wallet '{}': {}", req.from_wallet, e);
                HttpResponse::BadRequest().body(e.to_string())
            }
        }
    } else {
        warn!("POST /send - wallet '{}' not found", req.from_wallet);
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
    info!("GET /tx/{}/{}", walletid, txid);
    if let Some(client) = data.clients.get(walletid.as_str()) {
        let txid = match Txid::from_str(&txid) {
            Ok(id) => id,
            Err(e) => {
                warn!("Invalid txid format '{}': {}", txid, e);
                return HttpResponse::BadRequest().body(format!("Invalid transaction ID: {}", e))
            }
        };

        match client.get_transaction(&txid, None) {
            Ok(tx) => HttpResponse::Ok().json(GetTransactionResultWrapper(tx)),
            Err(e) => {
                error!("Transaction '{}' not found for wallet '{}': {}", txid, walletid, e);
                HttpResponse::NotFound().body(e.to_string())
            }
        }
    } else {
        warn!("GET /tx - no active clients for wallet '{}'", walletid);
        HttpResponse::ServiceUnavailable().body("No active clients")
    }
}

async fn get_mempool_entry(data: web::Data<AppState>, path: web::Path<(String, String)>) -> impl Responder {
    let (walletid, txid) = path.into_inner();
    info!("GET /mempool/{}/{}", walletid, txid);
    if let Some(client) = data.clients.get(walletid.as_str()) {
        let txid = match Txid::from_str(&txid) {
            Ok(id) => id,
            Err(e) => {
                warn!("Invalid txid format '{}': {}", txid, e);
                return HttpResponse::BadRequest().body(format!("Invalid transaction ID: {}", e))
            }
        };

        match client.get_mempool_entry(&txid) {
            Ok(entry) => HttpResponse::Ok().json(entry),
            Err(e) => {
                error!("Mempool entry '{}' not found for wallet '{}': {}", txid, walletid, e);
                HttpResponse::NotFound().body(e.to_string())
            }
        }
    } else {
        warn!("GET /mempool - no active clients for wallet '{}'", walletid);
        HttpResponse::ServiceUnavailable().body("No active clients")
    }
}

pub async fn run_server() -> std::io::Result<()> {
    // Initialize logger with a sensible default so logs appear in Docker even if RUST_LOG is not set
    let env = env_logger::Env::default().default_filter_or("info,actix_web=info");
    env_logger::Builder::from_env(env).init();

    let config = Config::from_env().expect("Failed to load config");
    info!(
        "Starting server with config: server_url={}, rpc_url={}",
        config.server_url, config.rpc_url
    );
    let server_url = config.server_url.clone();
    let app_state = web::Data::new(AppState {
        config,
        clients: DashMap::new(),
    });

    // Bind to all interfaces so the service is reachable when running inside Docker
    let bind_addr = "0.0.0.0:8021";
    info!("Binding HTTP server at {}", bind_addr);
    HttpServer::new(move || {
        let cors = Cors::default()
            .allowed_origin(server_url.as_str())
            .allowed_methods(vec!["GET", "POST", "OPTIONS"])
            .allowed_headers(vec![header::AUTHORIZATION, header::ACCEPT, header::CONTENT_TYPE])
            .max_age(3600);
        App::new()
            .wrap(ActixLogger::default())
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
    .bind(bind_addr)?
    .run()
    .await
}

fn get_wallet(rpc: &Client, wallet_name: &str) -> bitcoincore_rpc::Result<LoadWalletResult> {
    info!("Checking wallet '{}' existence and loading/creating as needed", wallet_name);
    // Check if wallet exists
    let wallets = rpc.list_wallets()?;
    let wallet_exists = wallets.iter().any(|wallet| wallet == wallet_name);

    if wallet_exists {
        // Try loading the wallet
        match rpc.load_wallet(wallet_name) {
            Ok(result) => {
                info!("Wallet '{}' loaded", wallet_name);
                Ok(result)
            }
            Err(e) => {
                // If error is "already loaded" (code -4), unload and retry
                if e.to_string().contains("code: -4") {
                    warn!("Wallet '{}' already loaded. Reloading...", wallet_name);
                    rpc.unload_wallet(Some(wallet_name))?;
                    rpc.load_wallet(wallet_name)
                } else {
                    error!("Failed to load wallet '{}': {}", wallet_name, e);
                    Err(e)
                }
            }
        }
    } else {
        // Try creating a new wallet
        info!("Creating new wallet '{}'", wallet_name);
        rpc.create_wallet(wallet_name, None, None, None, None)
            .map_err(|e| {
                if e.to_string().contains("code: -4") {
                    error!("Wallet '{}' already exists but was not listed", wallet_name);
                    RpcError::ReturnedError("Wallet already exists but was not listed".into())
                } else {
                    e
                }
            })
    }
}
