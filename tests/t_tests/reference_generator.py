"""Independent reference provenance; mpmath 1.3.0, 80 decimal digits."""
import mpmath as m
m.mp.dps = 80
for df in [m.mpf(50) / 17, m.mpf(4)]:
    t = 4 / m.sqrt(m.mpf(5) / 3)
    def tail(x):
        return m.betainc(df / 2, m.mpf('.5'), 0, df / (df + x*x), regularized=True)
    print('df:', df, 'p:', tail(t), 'critical:', m.findroot(lambda x: tail(x) - m.mpf('.05'), 3))
for df, t in [(m.mpf(49)/17, 4/m.sqrt(m.mpf(7)/3)), (m.mpf(3), m.mpf(12)/5)]:
    def tail(x):
        return m.betainc(df/2, m.mpf('.5'), 0, df/(df+x*x), regularized=True)
    print('unequal N df:', df, 'p:', tail(t), 'critical:', m.findroot(lambda x: tail(x)-m.mpf('.05'), 3))
