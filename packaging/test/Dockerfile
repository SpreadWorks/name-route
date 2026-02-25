FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    curl \
    procps \
    && rm -rf /var/lib/apt/lists/*

COPY nameroute_0.1.0_amd64.deb /tmp/nameroute.deb

# Install without systemd (postinst will skip systemctl)
RUN dpkg -i /tmp/nameroute.deb && rm /tmp/nameroute.deb

# Disable Docker polling and /etc/hosts management in test config
RUN sed -i '/^\[docker\]/,/^\[/{s/^enabled = true/enabled = false/}' /etc/nameroute/config.toml && \
    sed -i 's/^manage_hosts = true/manage_hosts = false/' /etc/nameroute/config.toml

COPY test.sh /test.sh
RUN chmod +x /test.sh

CMD ["/test.sh"]
