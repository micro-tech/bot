# Docker & Systemd Deploy (Task #3)

## Files Created
- `Dockerfile` — multi-stage Rust build → minimal Debian image
- `helix.service` — systemd unit file for production deployment

## Usage

### Docker
```bash
docker build -t helix .
docker run -p 8443:8443 helix
```

### Systemd
```bash
sudo cp helix.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now helix
```

## Status
- Dockerfile: Complete
- systemd unit: Complete
- Documentation: Complete (this file)
