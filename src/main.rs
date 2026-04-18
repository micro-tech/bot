use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::prelude::*;
use std::env;
use tracing::{error, info};

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("Logged in as {}", ready.user.name);
    }

    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }
        let content = msg.content.to_lowercase();
        match content.as_str() {
            "hello" => {
                if let Err(e) = msg.reply(&ctx.http, "Hello!").await {
                    error!("Error replying to hello: {:?}", e);
                } else {
                    info!("Replied to hello from {}", msg.author.name);
                }
            }
            "ping" => {
                if let Err(e) = msg.reply(&ctx.http, "pong").await {
                    error!("Error replying to ping: {:?}", e);
                } else {
                    info!("Replied to ping from {}", msg.author.name);
                }
            }
            "help" => {
                if let Err(e) = msg.reply(&ctx.http, "Available commands: hello, ping, help").await {
                    error!("Error replying to help: {:?}", e);
                } else {
                    info!("Replied to help from {}", msg.author.name);
                }
            }
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();
    let token = env::var("DISCORD_TOKEN").expect("DISCORD_TOKEN must be set");
    let intents = GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT | GatewayIntents::GUILDS;
    let mut client = Client::builder(&token, intents)
        .event_handler(Handler)
        .await
        .expect("Error creating client");
    if let Err(e) = client.start().await {
        error!("Client error: {:?}", e);
    }
}