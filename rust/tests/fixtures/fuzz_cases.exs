import NimbleParsec

# Parses each line of stdin with a datetime grammar that mirrors the Rust
# `datetime_parser` in differential_runner.rs, emitting one serialized result
# per line so the Rust side can compare against generated inputs.
defmodule FuzzFixtures do
  import NimbleParsec

  date =
    integer(4)
    |> ignore(string("-"))
    |> integer(2)
    |> ignore(string("-"))
    |> integer(2)

  time =
    integer(2)
    |> ignore(string(":"))
    |> integer(2)
    |> ignore(string(":"))
    |> integer(2)

  defparsec :datetime, date |> ignore(string("T")) |> concat(time)

  def run do
    :stdio
    |> IO.stream(:line)
    |> Enum.each(fn line ->
      input = String.trim_trailing(line, "\n")

      case datetime(input) do
        {:ok, tokens, rest, _context, _line, offset} ->
          IO.puts("ok|#{rest}|#{offset}|#{length(tokens)}|#{format_tokens(tokens)}")

        {:error, reason, rest, _context, _line, offset} ->
          IO.puts("err|#{rest}|#{offset}|#{reason}")
      end
    end)
  end

  defp format_tokens(tokens), do: Enum.map_join(tokens, ",", &format_token/1)
  defp format_token(int) when is_integer(int), do: Integer.to_string(int)
  defp format_token(bin) when is_binary(bin), do: "s:" <> bin
end

FuzzFixtures.run()
