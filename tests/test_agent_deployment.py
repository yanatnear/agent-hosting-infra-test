import os
import time
import pytest

from conftest import get_deploy_timeout


class TestAgentDeployment:
    """Category 1: Agent Deployment - P0 Smoke Tests"""

    def test_create_agent_via_api_1_1(
        self,
        http_client: httpx.Client,
        api_url: str,
        unique_name: str,
        agent_image: str,
    ):
        """
        Test Case 1.1: Create a single agent via API — verify it reaches "running" state.

        This is the most basic sanity check for the agent hosting platform.
        """
        # Create agent
        create_response = http_client.post(
            f"{api_url}/instances",
            json={
                "name": unique_name,
                "image": agent_image,
            },
        )
        assert create_response.status_code == 201, (
            f"Expected 201 Created, got {create_response.status_code}: {create_response.text}"
        )

        # Poll until agent reaches "running" state
        timeout = get_deploy_timeout()
        start_time = time.time()
        final_state = None

        while time.time() - start_time < timeout:
            get_response = http_client.get(f"{api_url}/instances/{unique_name}")
            assert get_response.status_code == 200, (
                f"GET failed: {get_response.status_code}: {get_response.text}"
            )

            data = get_response.json()
            phase = data.get("phase")
            state = data.get("state")

            final_state = {"phase": phase, "state": state}

            if phase == "Running":
                break

            time.sleep(2)

        # Verify final state
        assert final_state["phase"] == "Running", (
            f"Agent did not reach 'Running' state within {timeout}s. Final state: {final_state}"
        )
        assert final_state["state"] == "running", (
            f"Agent state should be 'running', got: {final_state['state']}"
        )

        # Cleanup: delete the agent
        delete_response = http_client.delete(f"{api_url}/instances/{unique_name}")
        assert delete_response.status_code in (200, 202, 204), (
            f"Delete failed: {delete_response.status_code}: {delete_response.text}"
        )
