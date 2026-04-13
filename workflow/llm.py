"""LLM provider abstraction with per-agent conversation history and token tracking."""

from __future__ import annotations

import os
from dataclasses import dataclass, field

import anthropic
import openai


@dataclass
class LLMResponse:
    content: str
    input_tokens: int
    output_tokens: int


@dataclass
class ModelConfig:
    provider: str  # "anthropic" or "openai"
    model_id: str
    max_tokens: int = 8192
    temperature: float = 0.0


@dataclass
class TokenUsage:
    input_tokens: int = 0
    output_tokens: int = 0

    def add(self, resp: LLMResponse) -> None:
        self.input_tokens += resp.input_tokens
        self.output_tokens += resp.output_tokens

    def total(self) -> int:
        return self.input_tokens + self.output_tokens

    def to_dict(self) -> dict[str, int]:
        return {"input_tokens": self.input_tokens, "output_tokens": self.output_tokens}


@dataclass
class AgentLLM:
    """LLM client for a single agent, tracking conversation history and token usage."""

    config: ModelConfig
    system_prompt: str
    messages: list[dict[str, str]] = field(default_factory=list)
    usage: TokenUsage = field(default_factory=TokenUsage)

    def chat(self, user_message: str) -> LLMResponse:
        self.messages.append({"role": "user", "content": user_message})
        resp = _chat(self.config, self.system_prompt, self.messages)
        self.messages.append({"role": "assistant", "content": resp.content})
        self.usage.add(resp)
        return resp

    def reset(self) -> None:
        self.messages.clear()


def _get_provider(model_id: str) -> str:
    if model_id.startswith("claude"):
        return "anthropic"
    if model_id.startswith(("gpt", "o1", "o3", "o4")):
        return "openai"
    raise ValueError(f"Cannot infer provider for model: {model_id}")


def make_config(model_id: str, max_tokens: int = 8192, temperature: float = 0.0) -> ModelConfig:
    return ModelConfig(
        provider=_get_provider(model_id),
        model_id=model_id,
        max_tokens=max_tokens,
        temperature=temperature,
    )


def _chat(
    config: ModelConfig,
    system: str,
    messages: list[dict[str, str]],
) -> LLMResponse:
    if config.provider == "anthropic":
        return _chat_anthropic(config, system, messages)
    elif config.provider == "openai":
        return _chat_openai(config, system, messages)
    else:
        raise ValueError(f"Unknown provider: {config.provider}")


_anthropic_client: anthropic.Anthropic | None = None
_openai_client: openai.OpenAI | None = None


def _get_anthropic_client() -> anthropic.Anthropic:
    global _anthropic_client
    if _anthropic_client is None:
        api_key = os.environ.get("ANTHROPIC_API_KEY")
        if not api_key:
            raise RuntimeError("ANTHROPIC_API_KEY environment variable is not set")
        _anthropic_client = anthropic.Anthropic(api_key=api_key)
    return _anthropic_client


def _get_openai_client() -> openai.OpenAI:
    global _openai_client
    if _openai_client is None:
        api_key = os.environ.get("OPENAI_API_KEY")
        if not api_key:
            raise RuntimeError("OPENAI_API_KEY environment variable is not set")
        _openai_client = openai.OpenAI(api_key=api_key)
    return _openai_client


def _chat_anthropic(
    config: ModelConfig,
    system: str,
    messages: list[dict[str, str]],
) -> LLMResponse:
    client = _get_anthropic_client()
    resp = client.messages.create(
        model=config.model_id,
        max_tokens=config.max_tokens,
        temperature=config.temperature,
        system=system,
        messages=messages,
        timeout=300.0,
    )
    content = resp.content[0].text if resp.content else ""
    return LLMResponse(
        content=content,
        input_tokens=resp.usage.input_tokens,
        output_tokens=resp.usage.output_tokens,
    )


def _chat_openai(
    config: ModelConfig,
    system: str,
    messages: list[dict[str, str]],
) -> LLMResponse:
    client = _get_openai_client()
    oai_messages = [{"role": "system", "content": system}] + messages
    resp = client.chat.completions.create(
        model=config.model_id,
        max_tokens=config.max_tokens,
        temperature=config.temperature,
        messages=oai_messages,
    )
    content = resp.choices[0].message.content or ""
    usage = resp.usage
    return LLMResponse(
        content=content,
        input_tokens=usage.prompt_tokens if usage else 0,
        output_tokens=usage.completion_tokens if usage else 0,
    )
