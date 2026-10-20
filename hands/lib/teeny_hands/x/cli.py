"""The verbs of hands/x (docs/hands.md). Reads (R) go through the read cache
and are safe to expose to a drafting agent; acting verbs (A) only go through
`publish` and `media`, which run the guard and the posting rules themselves.

  R  get ID                 one post
  R  thread ID [--max N]    the post and its conversation
  R  user HANDLE            an account by its handle
  R  search QUERY [--max N] a recent search
  R  whoami                 the account's own id and username
  A  post                   stdin {"text", "media_ids"?, "gif_id"?, "quote_of"?}
  A  reply --to ID          stdin {"text", "media_ids"?, "gif_id"?}
  A  like ID                skipped under DRY_RUN
  A  unsay ID [--confirm]   delete an own post; previews unless confirmed
  A  upload --file PATH     from state/media/ or brain/assets/ only
"""

from __future__ import annotations

import argparse
from typing import Any, Mapping

from .. import hand
from . import cache, media, publish
from .client import number_setting
from .model import SearchOptions


def _get(args: argparse.Namespace, _: Any) -> Mapping[str, Any]:
    return {"post": cache.reads().get_post(args.id)}


def _thread(args: argparse.Namespace, _: Any) -> Mapping[str, Any]:
    reads = cache.reads()
    post = reads.get_post(args.id)
    if post is None:
        return {"post": None, "conversationId": None, "posts": []}
    conversation_id = post.get("conversationId") or args.id
    return {"post": post, "conversationId": conversation_id, "posts": reads.get_conversation(conversation_id, args.max)}


def _user(args: argparse.Namespace, _: Any) -> Mapping[str, Any]:
    return {"user": cache.reads().user_by_username(args.handle)}


def _search(args: argparse.Namespace, _: Any) -> Mapping[str, Any]:
    # A cost dial: X bills every post a search returns.
    wanted = args.max if args.max is not None else int(number_setting("X_SEARCH_MAX_RESULTS", 10, 10, 100))
    return {"posts": cache.reads().search_posts(args.query, SearchOptions(max_results=wanted))}


def _whoami(args: argparse.Namespace, _: Any) -> Mapping[str, Any]:
    return cache.reads().identity()


def _post(args: argparse.Namespace, body: Any) -> Mapping[str, Any]:
    return publish.post(body)


def _reply(args: argparse.Namespace, body: Any) -> Mapping[str, Any]:
    return publish.reply(args.to, body)


def _like(args: argparse.Namespace, _: Any) -> Mapping[str, Any]:
    return publish.like(args.id)


def _unsay(args: argparse.Namespace, _: Any) -> Mapping[str, Any]:
    return publish.unsay(args.id, confirm=args.confirm)


def _upload(args: argparse.Namespace, _: Any) -> Mapping[str, Any]:
    return media.upload(args.file)


def _max(value: str) -> int:
    number = int(value)
    if not 1 <= number <= 100:
        raise argparse.ArgumentTypeError("--max is 1..100")
    return number


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(prog="hands/x", description="The only holder of the X keys.")
    verbs = root.add_subparsers(dest="verb")

    def verb(name: str, handler: Any, help: str, body: bool = False) -> argparse.ArgumentParser:
        sub = verbs.add_parser(name, help=help)
        sub.set_defaults(handler=handler, takes_body=body)
        return sub

    verb("get", _get, "one post").add_argument("id")
    thread = verb("thread", _thread, "a post and its conversation")
    thread.add_argument("id")
    thread.add_argument("--max", type=_max, default=None)
    verb("user", _user, "an account by its handle").add_argument("handle")
    search = verb("search", _search, "a recent search")
    search.add_argument("query")
    search.add_argument("--max", type=_max, default=None)
    verb("whoami", _whoami, "the account's own id and username")
    verb("post", _post, "an own post or a quote (JSON on stdin)", body=True)
    verb("reply", _reply, "a reply (JSON on stdin)", body=True).add_argument("--to", required=True)
    verb("like", _like, "like a post").add_argument("id")
    unsay = verb("unsay", _unsay, "delete an own post")
    unsay.add_argument("id")
    unsay.add_argument("--confirm", action="store_true")
    verb("upload", _upload, "upload a file from state/media/ or brain/assets/").add_argument("--file", required=True)
    return root


def main(argv: list[str]) -> int:
    return hand.run("x", parser(), argv)
