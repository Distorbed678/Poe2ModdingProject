// Ports the RE work of an old build's Ghidra program onto a rebuilt exe (a hotfix) with the
// address map of scripts/re/addr_map.py: data types, the names (and namespaces) of every named
// function, their signatures and plate/repeatable comments, and the comments inside functions.
// Run it on the NEW program (the MCP's run_script_inline or the Script Manager), then save.
// Arguments: <addr_map.tsv> <old program path in the project>, for example
//   dumps/0.5.5e.hotfix1/addr_map.tsv /PathOfExileSteam.exe
// Functions outside .pdata (leaf functions, thunks) are placed with a neighbour's delta and
// accepted only when their first bytes are equal; what cannot be placed is listed. Comments
// outside functions are not ported: in the old program they were all analysis output (RTTI,
// imports), which the new program's analysis makes again. docs/re/README.md
//@category PoE2
import ghidra.app.script.GhidraScript;
import ghidra.app.cmd.function.*;
import ghidra.app.util.NamespaceUtils;
import ghidra.program.model.address.*;
import ghidra.program.model.data.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.*;
import java.io.*;
import java.util.*;

public class PortFromAddrMap extends GhidraScript {
	// old function start -> {new start, old size, 1 if moved unchanged}
	private TreeMap<Long, long[]> map = new TreeMap<>();
	private Program src, dst;
	private AddressSpace ss, ds;

	private boolean bytesEqual(long o, long n, int len) {
		try {
			byte[] a = new byte[len], b = new byte[len];
			return src.getMemory().getBytes(ss.getAddress(o), a) == len &&
				dst.getMemory().getBytes(ds.getAddress(n), b) == len && Arrays.equals(a, b);
		}
		catch (Exception e) {
			return false;
		}
	}

	private Long mapEntry(Function f) {
		long o = f.getEntryPoint().getOffset();
		int len = (int) Math.min(16, Math.max(1, f.getBody().getNumAddresses()));
		Map.Entry<Long, long[]> e = map.floorEntry(o);
		if (e != null && o < e.getKey() + e.getValue()[1]) {
			long n = o - e.getKey() + e.getValue()[0];
			return (o == e.getKey() || e.getValue()[2] == 1) ? n : null;
		}
		for (Map.Entry<Long, long[]> x : List.of(Optional.ofNullable(e), Optional.ofNullable(map.ceilingEntry(o)))
				.stream().flatMap(Optional::stream).toList()) {
			long n = o + (x.getValue()[0] - x.getKey());
			if (bytesEqual(o, n, len)) {
				return n;
			}
		}
		return null;
	}

	@Override
	public void run() throws Exception {
		String[] args = getScriptArgs();
		if (args.length < 2) {
			printerr("arguments: <addr_map.tsv> <old program path>");
			return;
		}
		try (BufferedReader r = new BufferedReader(new FileReader(args[0]))) {
			String l = r.readLine();
			while ((l = r.readLine()) != null) {
				String[] f = l.split("\t");
				if (!f[1].equals("-")) {
					map.put(Long.decode(f[0]), new long[] { Long.decode(f[1]), Long.parseLong(f[2]),
						f[4].equals("same") ? 1 : 0 });
				}
			}
		}
		dst = currentProgram;
		src = (Program) state.getProject().getProjectData().getFile(args[1]).getDomainObject(this,
			false, false, monitor);
		ss = src.getAddressFactory().getDefaultAddressSpace();
		ds = dst.getAddressFactory().getDefaultAddressSpace();
		try {
			DataTypeManager ddt = dst.getDataTypeManager();
			int before = ddt.getDataTypeCount(false);
			Iterator<DataType> it = src.getDataTypeManager().getAllDataTypes();
			while (it.hasNext()) {
				DataType t = it.next();
				if (!(t instanceof BuiltInDataType)) {
					ddt.resolve(t, DataTypeConflictHandler.KEEP_HANDLER);
				}
			}
			println("data types: " + before + " -> " + ddt.getDataTypeCount(false));

			FunctionManager dfm = dst.getFunctionManager();
			int named = 0, renamed = 0, already = 0, created = 0, sigs = 0, unplaced = 0;
			Map<Long, Long> entryMap = new HashMap<>();
			for (Function f : src.getListing().getFunctions(true)) {
				Long n = mapEntry(f);
				if (n != null) {
					entryMap.put(f.getEntryPoint().getOffset(), n);
				}
				if (f.getSymbol().getSource() == SourceType.DEFAULT || f.getName().startsWith("FUN_")) {
					continue;
				}
				named++;
				if (n == null) {
					unplaced++;
					println("unplaced " + f.getName(true) + " " + f.getEntryPoint());
					continue;
				}
				Address na = ds.getAddress(n);
				Function g = dfm.getFunctionAt(na);
				if (g == null && new CreateFunctionCmd(na).applyTo(dst)) {
					g = dfm.getFunctionAt(na);
					created++;
				}
				if (g == null) {
					println("cannot create " + f.getName(true) + " at " + na);
					continue;
				}
				Namespace ns = f.getParentNamespace().isGlobal() ? dst.getGlobalNamespace()
						: NamespaceUtils.createNamespaceHierarchy(f.getParentNamespace().getName(true),
							null, dst, SourceType.USER_DEFINED);
				if (g.getName(true).equals(f.getName(true))) {
					already++;
				}
				else {
					g.getSymbol().setNameAndNamespace(f.getName(), ns, SourceType.USER_DEFINED);
					renamed++;
				}
				if (f.getSignatureSource() != SourceType.DEFAULT &&
					new ApplyFunctionSignatureCmd(na, f.getSignature(), SourceType.USER_DEFINED).applyTo(dst)) {
					sigs++;
				}
				if (f.getComment() != null) {
					g.setComment(f.getComment());
				}
				if (f.getRepeatableComment() != null) {
					g.setRepeatableComment(f.getRepeatableComment());
				}
			}
			println("named functions " + named + ": already right " + already + ", renamed " + renamed +
				", created " + created + ", unplaced " + unplaced + "; signatures " + sigs);

			int set = 0, skipped = 0;
			AddressIterator ai = src.getListing().getCommentAddressIterator(src.getMemory(), true);
			while (ai.hasNext()) {
				Address a = ai.next();
				Function f = src.getFunctionManager().getFunctionContaining(a);
				if (f == null) {
					continue;
				}
				long o = f.getEntryPoint().getOffset();
				Long n = entryMap.get(o);
				Map.Entry<Long, long[]> e = map.floorEntry(o);
				boolean sameBody = e != null && (e.getValue()[2] == 1 || e.getKey() != o);
				if (n == null || !sameBody) {
					skipped++;
					continue;
				}
				Address b = ds.getAddress(a.getOffset() - o + n);
				for (CommentType t : CommentType.values()) {
					String c = src.getListing().getComment(t, a);
					if (c != null && !c.equals(dst.getListing().getComment(t, b))) {
						dst.getListing().setComment(b, t, c);
						set++;
					}
				}
			}
			println("comments set " + set + ", in unmapped functions " + skipped);
		}
		finally {
			src.release(this);
		}
	}
}
