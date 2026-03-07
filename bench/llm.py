"""LLM provider abstraction for Anthropic and OpenAI APIs."""

from __future__ import annotations

import os
from dataclasses import dataclass

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


def _get_provider(model_id: str) -> str:
    if model_id.startswith("claude"):
        return "anthropic"
    if model_id.startswith(("gpt", "o1", "o3", "o4")):
        return "openai"
    raise ValueError(f"Cannot infer provider for model: {model_id}")


def make_config(model_id: str) -> ModelConfig:
    return ModelConfig(provider=_get_provider(model_id), model_id=model_id)


def chat(
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


def _chat_anthropic(
    config: ModelConfig,
    system: str,
    messages: list[dict[str, str]],
) -> LLMResponse:
    client = anthropic.Anthropic(api_key=os.environ["ANTHROPIC_API_KEY"])
    resp = client.messages.create(
        model=config.model_id,
        max_tokens=config.max_tokens,
        temperature=config.temperature,
        system=system,
        messages=messages,
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
    client = openai.OpenAI(api_key=os.environ["OPENAI_API_KEY"])
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
