import json
from pathlib import Path


def calculate_percentile(values, percentage):
    """Match NumPy's default linear percentile interpolation."""
    ordered = sorted(values)
    position = (len(ordered) - 1) * percentage / 100
    lower = int(position)
    upper = min(lower + 1, len(ordered) - 1)
    fraction = position - lower
    return ordered[lower] + (ordered[upper] - ordered[lower]) * fraction


def format_results():
    phx_cu = []
    mfx_cu = []
    results_path = Path(__file__).with_name('benchmark_results.csv')
    for line in results_path.open():
        line = line.strip()
        [phx, mfx, num_orders, signature] = line.split(',')
        if phx == "PHX":
            continue
        if int(num_orders) != 0:
            phx_cu.append(float(phx) / int(num_orders))
            mfx_cu.append(float(mfx) / int(num_orders))

    result_list = []

    exchanges = [(phx_cu, 'PHX'), (mfx_cu, 'MFX')]
    percentiles = [50, 95, 99]
    for (cu, name) in exchanges:
        for percentile in percentiles:
            result = round(calculate_percentile(cu, percentile))
            result_list.append(
                {
                    "name": "{name}_{percentile}".format(
                        name=name,
                        percentile=percentile
                    ),
                    "value": result,
                    "unit": "CU",
                    "range": "",
                    "extra": "",
                }
            )
    print(json.dumps(result_list, indent=4))

if __name__ == "__main__":
    format_results()
