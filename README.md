# amibussy

A Rust application that integrates **Toggl Track** and **Telegram** via webhooks to automatically update your Telegram chat title based on your working status. 
Working from home and tired of distractions? 
Create a Telegram chat, subscribe your loved ones to it, define your boundaries, and let amibussy handle the rest.

---

## Table of Contents
- [Overview](#overview)
- [Features](#features)
- [Prerequisites](#prerequisites)
- [Installation](#installation)
- [Configuration](#configuration)
  - [Configuration Fields](#configuration-fields)
- [Usage](#usage)
- [Roadmap](#roadmap)
- [Contributing](#contributing)
- [License](#license)
- [Acknowledgements](#acknowledgements)

---

## Overview

**amibussy** is a Rust application designed to synchronize your **Toggl Track** time entries with your **Telegram** chat status. It listens for webhook events from Toggl Track and updates the title of a specified Telegram chat to reflect whether you’re working, on a break, or inactive after a period.

Using **Ngrok**, the application exposes a local server to the internet, allowing Toggl Track to send webhook events directly to your machine or server securely, even behind a NAT.

---

## Features

- **Almost Real-time Status Updates:** Automatically changes your Telegram chat title when you start or stop a time entry in Toggl Track.
- **AFK Detection:** Switches your status to “Not Working” if you've been inactive longer than a specified duration.
- **Customizable Titles:** Configure titles for statuses like "Busy", "On Break", and "Not Working".
- **Easy Configuration:** Simple setup using a YAML configuration file.
- **Async Performance:** Built with asynchronous Rust libraries for efficient performance.

---

## Prerequisites

- **Rust:** Install it via [rustup.rs](https://rustup.rs)
- **Ngrok Account:** Create an account at [ngrok.com](https://ngrok.com)
- **Telegram Bot:** Create a bot using [BotFather](https://t.me/BotFather) to obtain a bot toke
- **Toggl Track Account:** Sign up at [toggl.com/track](https://toggl.com/track)
- **Telegram Chat ID:** Obtain the ID of the chat where you want the title updates

---

## Installation

1. **Clone the Repository:**

   ```bash
   git clone https://github.com/m0n0x41d/amibussy.git
   cd amibussy
   ```

2.	Build the Application:

```
cargo build --release
```

This will compile the project in release mode, producing an optimized binary.

## Configuration

Create a configuration file at `~/.config/amibussy/settings.yaml` with the following content:

```
bot_token: "YOUR_TELEGRAM_BOT_TOKEN"
ngrok_authtoken: "YOUR_NGROK_AUTHTOKEN"
ngrok_domain: "YOUR_NGROK_DOMAIN"
chat_id: "YOUR_TELEGRAM_CHAT_ID"
busy_chat_status: "Busy"
break_chat_status: "On Break"
not_working_status: "Not Working"
minutes_till_afk: 15
```

A free Ngrok account is sufficient for amibussy but may have limitations. With a free account, you will still have access to one static Ngrok domain.

### Configuration Fields

	•	bot_token: The token provided by BotFather for your Telegram bot. Make sure to add the bot as an admin to your chat.
	•	ngrok_authtoken: Your Ngrok authentication token.
	•	ngrok_domain: A reserved domain from Ngrok.
	•	chat_id: The ID of the Telegram chat to update (e.g., @your_chat_id).
	•	busy_chat_status: The title when a time entry starts.
	•	break_chat_status: The title when a time entry stops.
	•	not_working_status: The title after being inactive for the specified AFK duration.
	•	minutes_till_afk: The number of minutes before switching to “Not Working”.

## Usage

1.	Run the Application:

```
cargo run --release
```


2.	Set Up Ngrok:
Ensure your Ngrok tunnel is set up correctly with the domain specified in your configuration. The application uses Ngrok’s Rust library to start the tunnel automatically.
3.	Configure Toggl Track Webhook:
Note: Webhook configuration is not automated yet. For now, you need to set up the webhook manually via Toggl Track’s API or web interface.
Example setup using curl:

```
curl -u <TOGGLTRACK_API_TOKEN>:api_token \
     -X POST \
     -d '{
       "url_callback": "<YOUR_NGROK_DOMAIN>/webhook",
       "event_filters": [{"entity": "time_entry", "action": "*"}],
       "enabled": true,
       "description": "Time entries watchdog"
     }' \
     -H 'User-Agent: curl' \
     -H 'Content-Type: application/json' \
     https://api.track.toggl.com/webhooks/api/v1/subscriptions/<YOUR_WORKSPACE_ID> | jq


curl -u <TOGGLTRACK_API_TOKEN>:api_token \
  https://api.track.toggl.com/webhooks/api/v1/subscriptions/<YOUR_WORKSPACE_ID>/<YOUR_WEBHOOK_SUBSCRIPTION_ID> | jq
  -H "Content-Type: application/json" \
  -d '{"enabled": true}'
```


4.	Enjoy Your Status Updates:

Track your time in Toggl Track and watch your Telegram chat title update accordingly!
It will work with both - simple timers and pomodoros.

## Roadmap

• Automated Webhook Configuration: Implement functionality to automatically manage webhooks.
• Enhanced Error Handling: Improve error messages and exception handling.
• Release Binaries: Build release binaries to simplify installation.
• Extended Configuration Options: Support environment variables and command-line arguments.
• Make it work as daemon, without redundant headaches.
• Unit Tests: Add tests to ensure code stability and reliability.

## Contributing

Contributions are welcome! If you have suggestions or encounter issues, please open an issue or submit a pull request.

## License

This project is licensed under the MIT License. See the LICENSE file for details.

## Acknowledgements

• Toggl Track: For time-tracking service and webhooks.
• Telegram Bot API: For enabling bot interactions.
• Ngrok: For secure tunnels to localhost.
• Axum: For the web framework used in this application.
• Tokio: For async runtime support.

