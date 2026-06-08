# Docker & Systemd Deploy (Task #3)

## Files Created
- `Dockerfile` — multi-stage Rust build → minimal Debian image
- `bot.service` — systemd unit file for production deployment

## Usage

### Docker
```bash
docker build -t bot .
docker run -p 8443:8443 bot
```

### Systemd
```bash
sudo cp bot.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now bot
```

## Status
- Dockerfile: Complete
- systemd unit: Complete
- Documentation: Complete (this file)
