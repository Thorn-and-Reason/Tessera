"""Independent NumPy references for development probe output; never shipped in packages.

Run with --binary target/release/examples/perf_probe --fixture PATH --output PATH.
Counts and point/source identities are exact; finite estimates use abs tolerance 1e-10.
"""
import argparse
from collections import Counter
import csv
import hashlib
import json
from pathlib import Path
import subprocess
import numpy as np


def identity(path):
    return {"path": str(path.resolve()), "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "bytes": path.stat().st_size}


def check(binary, fixture):
    with fixture.open(newline="") as source:
        reader = csv.reader(source)
        names = next(reader)
        raw = list(reader)
    columns = [[row[i] for row in raw] for i in range(len(names))]
    values = [[float(v) for v in col] if i < len(names)-1 else col
              for i, col in enumerate(columns)]
    numeric = np.array(values[:-1], dtype=np.float64)
    records = {}
    for operation in ("frequencies", "histogram", "scatter", "boxplot"):
        proc = subprocess.run([str(binary.resolve()), operation, str(fixture.resolve())],
                              capture_output=True, text=True, check=True)
        project = json.loads(proc.stdout)
        result = project["runs"][0]["analysis"][operation]
        checks = 0
        errors = []

        def exact(actual, expected):
            nonlocal checks
            assert actual == expected, (operation, actual, expected)
            checks += 1

        def near(actual, expected):
            nonlocal checks
            error = abs(actual - float(expected))
            assert error <= 1e-10, (operation, actual, expected, error)
            errors.append(error)
            checks += 1

        exact(result["source_rows"], len(raw))
        if operation == "frequencies":
            exact(len(result["tables"]), len(values))
            for i, table in enumerate(result["tables"]):
                reference = Counter(values[i])
                exact(table["column"], str(i + 1))
                exact(table["valid_n"], len(raw))
                exact(table["valid_rows"], len(raw))
                exact(table["missing_n"], 0)
                exact(len(table["categories"]), len(reference))
                cumulative = 0
                for row, (value, count) in zip(table["categories"], sorted(reference.items())):
                    cumulative += count
                    exact(row["value"], value)
                    exact(row["rows"], count)
                    exact(row["frequency"], count)
                    near(row["percent"]["value"], 100 * count / len(raw))
                    near(row["valid_percent"]["value"], 100 * count / len(raw))
                    exact(row["cumulative_frequency"], cumulative if i < len(values)-1 else None)
                    if i < len(values)-1:
                        near(row["cumulative_percent"]["value"], 100 * cumulative / len(raw))
        elif operation == "histogram":
            x = numeric[0]
            bins = min(128, 1 + (len(raw)-1).bit_length())
            counts, edges = np.histogram(x, bins=bins, range=(float(x.min()), float(x.max())))
            exact(result["n"], len(raw))
            exact(result["valid_rows"], len(raw))
            exact(len(result["bins"]), bins)
            for i, row in enumerate(result["bins"]):
                exact(row["frequency"], int(counts[i]))
                exact(row["rows"], int(counts[i]))
                near(row["left"], edges[i])
                near(row["right"], edges[i+1])
            exact(sum(b["frequency"] for b in result["bins"]), len(raw))
        elif operation == "scatter":
            exact(result["n"], len(raw))
            exact(result["valid_rows"], len(raw))
            for axis in range(2):
                exact(result["ranges"][axis][0]["value"], float(numeric[axis].min()))
                exact(result["ranges"][axis][1]["value"], float(numeric[axis].max()))
            k = min(2000, len(raw))
            exact(len(result["points"]), k)
            for i, point in enumerate(result["points"]):
                ordinal = i*(len(raw)-1)//(k-1) if len(raw)>k else i
                exact(point, {"row":ordinal,"x":values[0][ordinal],"y":values[1][ordinal],"frequency":1})
        else:
            groups = {}
            for i, key in enumerate(values[-1]):
                groups.setdefault(key, []).append(i)
            exact(len(result["groups"]), len(groups))
            outliers = []
            for index, (row, (key, ordinals)) in enumerate(zip(result["groups"], sorted(groups.items()))):
                x = numeric[0, ordinals]
                q = np.quantile(x, [0.25, 0.5, 0.75], method="linear")
                lower, upper = q[0]-1.5*(q[2]-q[0]), q[2]+1.5*(q[2]-q[0])
                inside = x[(x >= lower) & (x <= upper)]
                outside = [(i, values[0][i]) for i in ordinals if values[0][i]<lower or values[0][i]>upper]
                exact(row["value"], key)
                exact(row["rows"], len(x))
                exact(row["n"], len(x))
                for field, expected in zip(("minimum","q1","median","q3","maximum"),(x.min(),*q,x.max())):
                    near(row[field]["value"], expected)
                near(row["whiskers"][0]["value"], inside.min())
                near(row["whiskers"][1]["value"], inside.max())
                exact(row["outlier_rows"], len(outside))
                exact(row["outlier_n"], len(outside))
                outliers.extend({"group_index":index,"row":i,"value":value,"frequency":1}
                                for i,value in sorted(outside,key=lambda v:(v[1],v[0])))
            k = min(2000, len(outliers))
            expected = [outliers[i*(len(outliers)-1)//(k-1)] for i in range(k)] if len(outliers)>k else outliers
            exact(result["points"], expected)
        records[operation] = {"checks":checks,"max_absolute_error":max(errors,default=0.0),
                              "retained_analysis_bytes":int(proc.stderr.strip().split("=")[-1]),
                              "output_sha256":hashlib.sha256(proc.stdout.encode()).hexdigest(),
                              "all_checks_passed":True}
    return {"fixture":identity(fixture),"binary":identity(binary),"numpy":np.__version__,
            "method":"Full independent CSV cohort; Counter, NumPy histogram and linear/type-7 quantiles; exact source-ordinal reduction",
            "absolute_tolerance":1e-10,"counts_and_point_identities":"exact, every retained point checked",
            "operations":records}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary",type=Path,required=True)
    parser.add_argument("--fixture",type=Path,required=True)
    parser.add_argument("--output",type=Path,required=True)
    args=parser.parse_args()
    if args.output.exists():
        parser.error("Reference output exists; choose a new path.")
    result=check(args.binary,args.fixture)
    with args.output.open("x") as destination:
        json.dump(result,destination,indent=2)
        destination.write("\n")
    print(json.dumps(result["operations"]))


if __name__ == "__main__":
    main()
