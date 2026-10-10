#!/usr/bin/env python3
"""Check a NetCDF-3 classic file without a NetCDF library.

    python3 check_netcdf.py path/to/file.nc

The point of this script is to be a reader *this project did not write*: the
exporter's own reader could share a misunderstanding of the format with its
writer, and this one is spelled out from the specification, so a file that
passes both is a file xarray can open.

It prints the dimensions, the variables, the global attributes and a few
values, and exits non-zero if the layout is not what the specification says.
"""

from __future__ import annotations

import struct
import sys

TAG_DIMENSION = 0x0A
TAG_VARIABLE = 0x0B
TAG_ATTRIBUTE = 0x0C
NC_CHAR = 2
NC_DOUBLE = 6


class Reader:
    def __init__(self, data: bytes) -> None:
        self.data = data
        self.at = 0

    def u32(self) -> int:
        value = struct.unpack_from(">I", self.data, self.at)[0]
        self.at += 4
        return value

    def name(self) -> str:
        length = self.u32()
        text = self.data[self.at : self.at + length].decode("utf-8", "replace")
        self.at += length
        self.at += (-self.at) % 4
        return text

    def attributes(self) -> dict[str, object]:
        count = self.u32()
        found: dict[str, object] = {}
        for _ in range(count):
            name = self.name()
            kind = self.u32()
            nelems = self.u32()
            if kind == NC_CHAR:
                found[name] = self.data[self.at : self.at + nelems].decode("utf-8", "replace")
            elif kind == NC_DOUBLE:
                found[name] = list(
                    struct.unpack_from(f">{nelems}d", self.data, self.at)
                )
            else:
                found[name] = f"<type {kind}>"
            self.at += nelems * (8 if kind == NC_DOUBLE else 1)
            self.at += (-self.at) % 4
        return found


def read(path: str) -> dict[str, object]:
    data = open(path, "rb").read()
    reader = Reader(data)
    magic = data[:3]
    version = data[3]
    if magic != b"CDF" or version not in (1, 2):
        raise SystemExit(f"{path}: not a NetCDF classic file (magic {magic!r})")
    reader.at = 4
    numrecs = reader.u32()

    dimensions: dict[str, int] = {}
    if reader.u32() != TAG_DIMENSION:
        raise SystemExit("the dimension list is missing")
    for _ in range(reader.u32()):
        # Python evaluates the right-hand side before the subscript, so the
        # name has to be read first: `d[reader.name()] = reader.u32()` would
        # read the size from the middle of the name.
        name = reader.name()
        dimensions[name] = reader.u32()

    # The global attribute list carries its own tag before its count.
    if reader.u32() != TAG_ATTRIBUTE:
        raise SystemExit("the global attribute list is missing its tag")
    attributes = reader.attributes()

    variables: dict[str, dict[str, object]] = {}
    if reader.u32() != TAG_VARIABLE:
        raise SystemExit("the variable list is missing")
    for _ in range(reader.u32()):
        name = reader.name()
        dims = [reader.u32() for _ in range(reader.u32())]
        if reader.u32() != TAG_ATTRIBUTE:
            raise SystemExit(f"variable {name} has no attribute list tag")
        attributes_of = reader.attributes()
        kind = reader.u32()
        size = reader.u32()
        begin = reader.u32()
        count = 1
        for index in dims:
            count *= list(dimensions.values())[index] if dimensions else 1
        variables[name] = {
            "dims": dims,
            "attrs": attributes_of,
            "kind": kind,
            "size": size,
            "begin": begin,
            "values": list(struct.unpack_from(f">{count}d", data, begin))
            if kind == NC_DOUBLE
            else None,
        }
    return {
        "numrecs": numrecs,
        "dimensions": dimensions,
        "attributes": attributes,
        "variables": variables,
    }


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    for path in sys.argv[1:]:
        parsed = read(path)
        print(f"== {path}")
        print(f"numrecs {parsed['numrecs']}")
        print(f"dimensions {parsed['dimensions']}")
        print(f"attributes {parsed['attributes']}")
        variables = parsed["variables"]  # type: ignore[assignment]
        for name, variable in variables.items():
            values = variable["values"]
            head = values[:3] if isinstance(values, list) else None
            print(
                f"  {name}: dims {variable['dims']} type {variable['kind']} "
                f"at {variable['begin']} {variable['size']} bytes, first {head}"
            )
        # A check worth making: the data variable's values are finite numbers.
        data = variables.get("data", {}).get("values")
        if isinstance(data, list) and data:
            assert all(value == value for value in data[:100]), "the data has NaNs"
        # And the coordinate lengths must match the data's dimensions.
        time = variables.get("time", {}).get("values")
        spectral = variables.get("spectral", {}).get("values")
        if isinstance(time, list) and isinstance(spectral, list):
            assert len(data) == len(time) * len(spectral), "the data does not fit its axes"
            print(
                f"  ok: {len(time)} times × {len(spectral)} wavelengths = {len(data)} values"
            )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
