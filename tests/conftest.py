import os
import uuid
import pytest
import httpx
import kubernetes


def get_api_url() -> str:
    url = os.environ.get("AGENT_API_URL")
    if not url:
        raise RuntimeError("AGENT_API_URL environment variable must be set")
    return url


def get_namespace() -> str:
    return os.environ.get("AGENT_NAMESPACE", "agents")


def get_deploy_timeout() -> int:
    return int(os.environ.get("DEPLOY_TIMEOUT_SECONDS", "120"))


@pytest.fixture(scope="session")
def api_url() -> str:
    return get_api_url()


@pytest.fixture(scope="session")
def http_client() -> httpx.Client:
    with httpx.Client(timeout=30.0) as client:
        yield client


@pytest.fixture(scope="session")
def k8s_client() -> kubernetes.client.CoreV1Api:
    kubeconfig = kubernetes.config.load_kube_config()
    return kubernetes.client.CoreV1Api()


@pytest.fixture
def unique_name() -> str:
    return f"test-{uuid.uuid4().hex[:8]}"


@pytest.fixture
def agent_image() -> str:
    return os.environ.get("TEST_AGENT_IMAGE", "alpine:latest")
