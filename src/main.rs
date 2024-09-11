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

fn get_service_name(key: &str) -> Option<&str> {
    SERVICE_NAMES.get(key).copied()
}

async fn index() -> Html<String> {
    let context = Context::new();
    Html(TERA.render("index.html", &context).unwrap())
}

async fn list_services() -> Html<String> {
    // TODO: local proxy for service discovery
    let coap = Coap::new();
    let services = coap.service_discovery(None, None).await.unwrap();

    let mut context = Context::new();
    let services: Vec<_> = services.iter().map(|s| {
            (&s.0, get_service_name(&s.0).unwrap_or(&s.0), &s.1, s.2)
        })
        .collect();
    context.insert("services", &services);
    Html(TERA.render("services.html", &context).unwrap())
}

#[derive(Debug, serde::Deserialize)]
struct Rgbw {
    r: u8,
    g: u8,
    b: u8,
    w: u8,
}

async fn service(Path(service_id): Path<String>, request: Request) -> Html<String> {
    // TODO: stored type in local proxy
    let coap = Coap::new();
    let service = coap.service_discovery(Some(&service_id), None).await.unwrap();
    // TODO: retry a few times if empty

    if service.len() != 1 {
        return Html("Error: could not discover this service".to_string());
    }

    let service = &service[0];
    let id = &service.0;
    let name = get_service_name(id).unwrap_or(id);
    let ser_type = if let Some(ser_type) = &service.1 {
        ser_type
    } else {
        return Html(format!("Error: missing type for the discovered service {}", id));
    };
    let addr = &service.2;

    match ser_type.as_str() {
        "rgbw" => service_rgbw(id, name, addr, request).await,
        _ => return Html(format!("Error: unknown service type {}", ser_type)),
    }
}

async fn service_rgbw(id: &str, name: &str, addr: &SocketAddr, request: Request) -> Html<String> {
    let mut context = Context::new();
    context.insert("name", name);

    let coap = Coap::new();

    if let Ok(Form(rgbw)) = request.extract::<Form<Rgbw>, _>().await {
        let payload_map = ciborium::value::Value::Map([
                    (ciborium::value::Value::Text("r".to_string()), ciborium::value::Value::Integer(rgbw.r.into())),
                    (ciborium::value::Value::Text("g".to_string()), ciborium::value::Value::Integer(rgbw.g.into())),
                    (ciborium::value::Value::Text("b".to_string()), ciborium::value::Value::Integer(rgbw.b.into())),
                    (ciborium::value::Value::Text("w".to_string()), ciborium::value::Value::Integer(rgbw.w.into())),
                    (ciborium::value::Value::Text("d".to_string()), ciborium::value::Value::Integer(3000.into())),
                ].to_vec());

        let _ = coap.set(addr, id, &payload_map).await;
        context.insert("r", &rgbw.r);
        context.insert("g", &rgbw.g);
        context.insert("b", &rgbw.b);
        context.insert("w", &rgbw.w);
    } else {
        let data = coap.get(addr, id, None).await;

        let data = if let Ok(data) = data {
            data
        } else {
            return Html(format!("Error: Problem with retrieveng data from {}", id));
        };

        let data = if let Some(data) = data {
            data
        } else {
            return Html(format!("Error: Missing content format sent by {}", id));
        };

        let data = if let Content::Cbor(data) = data {
            data
        } else {
            return Html(format!("Error: Unexpected content type sent by {}", id));
        };

        let data = if let ciborium::value::Value::Map(data) = data {
            data
        } else {
            return Html(format!("Error: Unexpected CBOR format sent by {}", id));
        };

        for channel in ["r", "g", "b", "w"] {
            if let Some(value) = cbor_map_get(&data, channel) {
                context.insert(channel, value);
            } else {
                return Html(format!("Error: Missing value for parameter {} sent by {}", channel, id));
            }
        }
    }

    Html(TERA.render("services/rgbw.html", &context).unwrap())
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

    let app = Router::new()
        .route("/", get(index))
        .route("/service/:id", get(service))
        .route("/service/:id", post(service))
        .route("/services", get(list_services));

    let listener = tokio::net::TcpListener::bind("[::0]:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
