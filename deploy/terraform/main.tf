terraform {
  required_version = ">= 1.5"
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 5.0"
    }
  }
}

provider "google" {
  project = var.project_id
  region  = var.region
  zone    = var.zone
}

variable "project_id" {
  description = "GCP project ID"
  type        = string
}

variable "region" {
  description = "GCP region"
  type        = string
  default     = "us-central1"
}

variable "zone" {
  description = "GCP zone"
  type        = string
  default     = "us-central1-a"
}

variable "server_count" {
  description = "Number of K3s server nodes"
  type        = number
  default     = 1
}

variable "agent_count" {
  description = "Number of K3s agent (worker) nodes"
  type        = number
  default     = 2
}

variable "server_machine_type" {
  description = "Machine type for K3s server nodes"
  type        = string
  default     = "e2-standard-4"
}

variable "agent_machine_type" {
  description = "Machine type for K3s agent nodes"
  type        = string
  default     = "e2-standard-8"
}

variable "disk_size_gb" {
  description = "Boot disk size in GB"
  type        = number
  default     = 200
}

variable "ssh_public_key" {
  description = "SSH public key for VM access"
  type        = string
}

variable "ssh_user" {
  description = "SSH username"
  type        = string
  default     = "deploy"
}

# Network
resource "google_compute_network" "agents" {
  name                    = "agents-network"
  auto_create_subnetworks = false
}

resource "google_compute_subnetwork" "agents" {
  name          = "agents-subnet"
  ip_cidr_range = "10.0.0.0/16"
  network       = google_compute_network.agents.id
}

resource "google_compute_firewall" "internal" {
  name    = "agents-internal"
  network = google_compute_network.agents.name

  allow {
    protocol = "tcp"
  }
  allow {
    protocol = "udp"
  }
  allow {
    protocol = "icmp"
  }

  source_ranges = ["10.0.0.0/16"]
}

resource "google_compute_firewall" "external" {
  name    = "agents-external"
  network = google_compute_network.agents.name

  allow {
    protocol = "tcp"
    ports    = ["22", "80", "443", "6443", "8080"]
  }

  source_ranges = ["0.0.0.0/0"]
  target_tags   = ["agents-node"]
}

# K3s Server Nodes
resource "google_compute_instance" "k3s_server" {
  count        = var.server_count
  name         = "k3s-server-${count.index}"
  machine_type = var.server_machine_type
  tags         = ["agents-node", "k3s-server"]

  boot_disk {
    initialize_params {
      image = "ubuntu-os-cloud/ubuntu-2404-lts-amd64"
      size  = var.disk_size_gb
      type  = "pd-ssd"
    }
  }

  network_interface {
    subnetwork = google_compute_subnetwork.agents.id
    access_config {} # Ephemeral public IP
  }

  metadata = {
    ssh-keys = "${var.ssh_user}:${var.ssh_public_key}"
  }

  metadata_startup_script = <<-EOF
    #!/bin/bash
    set -euo pipefail
    apt-get update && apt-get install -y curl
    # K3s server install is handled by deploy scripts
  EOF
}

# K3s Agent (Worker) Nodes
resource "google_compute_instance" "k3s_agent" {
  count        = var.agent_count
  name         = "k3s-agent-${count.index}"
  machine_type = var.agent_machine_type
  tags         = ["agents-node", "k3s-agent"]

  boot_disk {
    initialize_params {
      image = "ubuntu-os-cloud/ubuntu-2404-lts-amd64"
      size  = var.disk_size_gb
      type  = "pd-ssd"
    }
  }

  network_interface {
    subnetwork = google_compute_subnetwork.agents.id
    access_config {}
  }

  metadata = {
    ssh-keys = "${var.ssh_user}:${var.ssh_public_key}"
  }
}

# Outputs
output "server_ips" {
  value = [for i in google_compute_instance.k3s_server : {
    name       = i.name
    internal   = i.network_interface[0].network_ip
    external   = i.network_interface[0].access_config[0].nat_ip
  }]
}

output "agent_ips" {
  value = [for i in google_compute_instance.k3s_agent : {
    name       = i.name
    internal   = i.network_interface[0].network_ip
    external   = i.network_interface[0].access_config[0].nat_ip
  }]
}
