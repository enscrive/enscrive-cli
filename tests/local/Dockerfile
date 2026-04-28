# Local-developer playground for testing `enscrive init --mode self-managed`.
# Fedora-based to match the EC2 Lane Fedora test (tests/ec2/lane-fedora-self-managed.*).
# Runs Docker-in-Docker so postgres/keycloak/qdrant/loki/vector/grafana
# brought up by `enscrive init` land on the playground's OWN dockerd, NOT
# your host's — that's how we sidestep port conflicts with your existing
# `enscrive-deploy provision --target dev` stack.
#
# Build:   docker build -t enscrive-playground tests/local
# Run:     tests/local/run.sh
FROM fedora:42

RUN dnf -y install --setopt=install_weak_deps=False \
        curl sudo procps-ng iproute iptables findutils tar gzip xz which jq \
        ca-certificates dnf-plugins-core \
    && dnf -y install docker-ce docker-ce-cli containerd.io docker-compose-plugin \
        || (dnf config-manager addrepo --from-repofile=https://download.docker.com/linux/fedora/docker-ce.repo && \
            dnf -y install docker-ce docker-ce-cli containerd.io docker-compose-plugin) \
    && dnf clean all

# Non-root test user. The default end-user persona — matches what a
# developer-evaluating-Enscrive runs as on their own laptop.
RUN useradd -m -u 1000 -G wheel tester \
    && groupadd -f docker \
    && usermod -aG docker tester \
    && echo "tester ALL=(ALL) NOPASSWD: ALL" > /etc/sudoers.d/99-tester

COPY entrypoint.sh /usr/local/bin/entrypoint.sh
COPY playground.sh /usr/local/bin/playground.sh
RUN chmod +x /usr/local/bin/entrypoint.sh /usr/local/bin/playground.sh

ENV PATH=/home/tester/.local/bin:/usr/local/bin:/usr/bin:/bin

# DinD writes to /var/lib/docker; declare it so an anonymous volume backs it
# (avoids overlay-on-overlay weirdness with the host filesystem).
VOLUME /var/lib/docker

# Container needs --privileged for inner dockerd to set up cgroups + iptables.
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["bash"]
