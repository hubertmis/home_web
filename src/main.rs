mod service_discovery;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::LazyLock;

use axum::{
    extract::{
        Path,
        Request,
    },
    Form,
    response::Html,
    routing::{get, post},
    RequestExt,
    Router,
};

use home_mng::{Coap, Content};

use tera::{Context, Tera};

#[derive(Debug)]
enum Error {
    InvalidResponse(home_mng::Error),
    MissingContentType,
    UnexpectedContentType,
    UnexpectedCborElement,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::InvalidResponse(e) => write!(f, "Invalid response: {}", e),
            Error::MissingContentType => write!(f, "Missing content type"),
            Error::UnexpectedContentType => write!(f, "Unexpected content type"),
            Error::UnexpectedCborElement => write!(f, "Unexpected CBOR element"),
        }
    }
}

impl std::error::Error for Error {}

static TERA: LazyLock<Tera> = LazyLock::new(|| {
    match Tera::new("templates/**/*.html") {
        Ok(t) => t,
        Err(e) => {
            println!("Parsing error: {}", e);
            ::std::process::exit(1);
        }
    }
});

static SERVICE_NAMES: LazyLock<HashMap<&str, &str>> = LazyLock::new(|| {
    let mut map = HashMap::new();
    map.insert("ap", "Ventilation system");
    map.insert("bac", "Bedroom air conditioner");
    map.insert("bbl", "Bedroom lights over the bed");
    map.insert("bbsw", "Bedroom light switch over the bed");
    map.insert("br", "Bedroom shades");
    map.insert("bwl", "Bedroom lights by the wardrobe");
    map.insert("bwsw", "Bedroom light switch by the wardrobe");
    map.insert("dac", "Dining room air conditioner");
    map.insert("dr1", "Dining room left shades");
    map.insert("dr2", "Dining room center shades");
    map.insert("dr3", "Dining room right shades");
    map.insert("drl", "Dining room lights");
    map.insert("drs", "Dining room light switch");
    map.insert("gbr", "Guest bathroom temperature controller");
    map.insert("gbrfh", "Guest bathroom floor heating");
    map.insert("hb", "Guest bathroom temperature valve");
    map.insert("k", "Kitchen shades");
    map.insert("kfh", "Kitchen floor heating");
    map.insert("kt", "Kitchen temperature controller");
    map.insert("lac", "Living room air conditioner");
    map.insert("ll", "Living room lights");
    map.insert("lr", "Living room shades");
    map.insert("ls", "Living room light switch");
    map.insert("oac", "Office air conditioner");
    map.insert("prx", "Proxy");
    map
});

static SERVICE_DISCOVERY: LazyLock<service_discovery::Proxy> = LazyLock::new(|| {
    service_discovery::Proxy::new()
});

fn get_service_name(key: &str) -> Option<&str> {
    SERVICE_NAMES.get(key).copied()
}

async fn index() -> Html<String> {
    let context = Context::new();
    Html(TERA.render("index.html", &context).unwrap())
}

async fn list_services() -> Html<String> {
    let services = SERVICE_DISCOVERY.all();

    let mut context = Context::new();
    let mut services: Vec<_> = services.iter().map(|s| {
            (&s.0, get_service_name(&s.0).unwrap_or(&s.0), &s.1, s.2)
        })
        .collect();
    services.sort_by_key(|k| (k.2, k.1));
    context.insert("services", &services);
    Html(TERA.render("services.html", &context).unwrap())
}

async fn service(Path(service_id): Path<String>, request: Request) -> Html<String> {
    let service = if let Some(service) = SERVICE_DISCOVERY.service(&service_id) {
        service
    } else {
        return Html(format!("Error: could not disocover this service"));
    };
    let id = &service_id;
    let name = get_service_name(id).unwrap_or(id);
    let ser_type = if let Some(ser_type) = &service.0 {
        ser_type
    } else {
        return Html(format!("Error: missing type for the discovered service {}", id));
    };
    let addr = &service.1;

    match ser_type.as_str() {
        "rgbw" => service_rgbw(id, name, addr, request).await,
        "shcnt" => service_shcnt(id, name, addr, request).await,
        _ => return Html(format!("Error: unknown service type {}", ser_type)),
    }
}

#[derive(Debug, serde::Deserialize)]
struct Rgbw {
    rgb: String,
    w: u8,
}

impl Rgbw {
    fn r(&self) -> u8 {
        self.channel(0)
    }
    fn g(&self) -> u8 {
        self.channel(1)
    }
    fn b(&self) -> u8 {
        self.channel(2)
    }

    fn channel(&self, idx: usize) -> u8 {
        let hex_str = self.rgb.trim_start_matches('#');
        let offset_start = 2*idx;
        let offset_end = 2*idx + 2;
        let channel_value = &hex_str[offset_start..offset_end];
        u8::from_str_radix(channel_value, 16).expect("Invalid rgbw value")
    }
}

async fn service_rgbw(id: &str, name: &str, addr: &SocketAddr, request: Request) -> Html<String> {
    let mut context = Context::new();
    context.insert("name", name);

    let coap = Coap::new();

    if let Ok(Form(rgbw)) = request.extract::<Form<Rgbw>, _>().await {
        let payload_map = ciborium::value::Value::Map([
                    (ciborium::value::Value::Text("r".to_string()), ciborium::value::Value::Integer(rgbw.r().into())),
                    (ciborium::value::Value::Text("g".to_string()), ciborium::value::Value::Integer(rgbw.g().into())),
                    (ciborium::value::Value::Text("b".to_string()), ciborium::value::Value::Integer(rgbw.b().into())),
                    (ciborium::value::Value::Text("w".to_string()), ciborium::value::Value::Integer(rgbw.w.into())),
                    (ciborium::value::Value::Text("d".to_string()), ciborium::value::Value::Integer(3000.into())),
                ].to_vec());

        let _ = coap.set(addr, id, &payload_map).await;
        context.insert("rgb", &rgbw.rgb);
        context.insert("w", &rgbw.w);
    } else {
        let data = coap.get(addr, id, None).await;

        let data = match extract_cbor_map_from_coap_response(data) {
            Ok(data) => data,
            Err(e) => return Html(format!("Error: {} in message received from {}", e, id)),
        };

        let mut rgb = "#".to_string();
        for channel in ["r", "g", "b"] {
            if let Some(value) = cbor_map_get(&data, channel) {
                let byte: u8 = value.as_integer().unwrap().try_into().expect(&format!("Invalid parameter {} sent by {}", channel, id));
                rgb += &format!("{:02x}", byte);
            } else {
                return Html(format!("Error: Missing value for parameter {} sent by {}", channel, id));
            }
        }

        context.insert("rgb", &rgb);

        if let Some(value) = cbor_map_get(&data, "w") {
            context.insert("w", value);
        } else {
            return Html(format!("Error: Missing value for parameter \"w\" sent by {}", id));
        }
    }

    Html(TERA.render("services/rgbw.html", &context).unwrap())
}

#[derive(Debug, serde::Deserialize)]
struct Shcnt {
    pos: u8,
}

async fn service_shcnt(id: &str, name: &str, addr: &SocketAddr, request: Request) -> Html<String> {
    let mut context = Context::new();
    context.insert("name", name);

    let coap = Coap::new();

    if let Ok(Form(shcnt)) = request.extract::<Form<Shcnt>, _>().await {
        let payload_map = ciborium::value::Value::Map([
                    (ciborium::value::Value::Text("val".to_string()), ciborium::value::Value::Integer(shcnt.pos.into())),
                ].to_vec());

        let _ = coap.set(addr, id, &payload_map).await;
        context.insert("pos", &shcnt.pos);
    } else {
        let data = coap.get(addr, id, None).await;

        let data = match extract_cbor_map_from_coap_response(data) {
            Ok(data) => data,
            Err(e) => return Html(format!("Error: {} in message received from {}", e, id)),
        };

        if let Some(value) = cbor_map_get(&data, "r") {
            context.insert("pos", value);
        } else {
            return Html(format!("Error: Missing value for parameter \"pos\" sent by {}", id));
        }
    }

    Html(TERA.render("services/shcnt.html", &context).unwrap())
}

fn extract_cbor_map_from_coap_response(response: Result<Option<Content>, std::io::Error>) -> Result<Vec<(ciborium::Value, ciborium::Value)>, Error> {
    let data = response.map_err(|e| Error::InvalidResponse(e))?; 
    let data = data.ok_or(Error::MissingContentType)?;
    let Content::Cbor(data) = data else {
        return Err(Error::UnexpectedContentType);
    };
    let ciborium::value::Value::Map(data) = data else {
        return Err(Error::UnexpectedCborElement);
    };
    Ok(data)
}

fn cbor_map_get<'a>(map: &'a Vec<(ciborium::value::Value, ciborium::value::Value)>, key: &str) -> Option<&'a ciborium::value::Value> {
    for entry in map {
        if let ciborium::value::Value::Text(entry_key) = &entry.0 {
            if entry_key == key {
                return Some(&entry.1);
            }
        }
    }
    None
}

#[tokio::main]
async fn main() {
    tokio::spawn(SERVICE_DISCOVERY.run());

    let app = Router::new()
        .route("/", get(index))
        .route("/service/:id", get(service))
        .route("/service/:id", post(service))
        .route("/services", get(list_services));

    let listener = tokio::net::TcpListener::bind("[::0]:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
