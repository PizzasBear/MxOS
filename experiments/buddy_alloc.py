from typing import Reversible, Union

ORDER: int = 9
MEMORY_SIZE: int = 2 << ORDER - 1


class LinkedListNode:
    __slots__ = 'data', 'next'

    def __init__(self, data, next_ = None):
        self.data = data
        self.next = next_

    def pop_next(self):
        "Pops the next element and retains all the elements after"
        if self.next is not None:
            next_data = self.next.data
            self.next = self.next.next
            return next_data
        else:
            return None

    def slice_next(self):
        "Slices off all the elements after the current one and returns them as a new linked list"
        next_ = self.next
        self.next = None
        return LinkedListNode(next_)


class LinkedList:
    __slots__ = 'root'

    def __init__(self, iterable: Union[Reversible, LinkedListNode] = ()):
        if isinstance(iterable, LinkedListNode):
            self.root = iterable
        else:
            node = None
            for arg in reversed(iterable):
                node = LinkedListNode(arg, node)
            self.root = node

    def push(self, data):
        "Pushes an element to the front of the linked list"
        self.root = LinkedListNode(data, self.root)

    def clear(self):
        "Removes all the elements of the linked list"
        self.root = None

    def pop(self):
        "Pops the first element of the linked list"

        if self.root is not None:
            root_data = self.root.data
            self.root = self.root.next
            return root_data
        else:
            return None

    def __iter__(self):
        node = self.root
        while node is not None:
            yield node.data
            node = node.next

    def __str__(self):
        return "LinkedList([{}])".format(", ".join(map(str, self)))

    def __repr__(self):
        return str(self)


memory = [0] * MEMORY_SIZE

buddies = [[True] * (MEMORY_SIZE >> i) for i in range(ORDER - 1)]
buddies.append([False] * (MEMORY_SIZE >> ORDER - 1))

free_areas = [LinkedList() for _ in range(ORDER - 1)]
free_areas.append(LinkedList(range(0, MEMORY_SIZE, 1 << ORDER - 1)))

def print_buddies():
    global buddies, free_areas

    for i, b in enumerate(buddies):
        print(end=' ' * ((1 << i) - 1))
        print(*map(lambda x: '#' if x else '.', b), sep=' ' * ((1 << i + 1) - 1))
    print(*free_areas, sep='\n')

def order_malloc(order: int) -> int:
    global buddies, free_areas

    order_free_areas = free_areas[order]
    order_buddies = buddies[order]
    while True:
        area = order_free_areas.pop()
        if area is None:
            if order + 1 < len(buddies):
                addr = order_malloc(order + 1)
                # order_buddies[addr >> order] = True
                order_buddies[addr >> order | 1] = False
                order_free_areas.push(addr | 1 << order)
                return addr
            else:
                raise RuntimeError
        elif not order_buddies[area >> order]:
            order_buddies[area >> order] = True
            return area

# def malloc(size: int) -> int:
#     global buddies, free_areas
# 
#     order = 0
#     while 1 << order < size:
#         order += 1
#     free_addr = addr = order_malloc(order)
#     for i in reversed(range(order)):
#         free_addr += 1 << i
#         if -size & 1 << i:
#             buddies[i][free_addr >> i] ^= True
#             free_areas[i].add(free_addr)
#             free_addr -= 1 << i
#     return addr

def order_free(addr: int, order: int, clear: bool = True):
    global buddies, free_areas

    oaddr = addr >> order
    buddy = oaddr ^ 1
    size = 1 << order
    order_buddies = buddies[order]

    assert order_buddies[oaddr]

    if clear:
        memory[addr:addr+size] = [0] * size

    if order + 1 < len(buddies) and not order_buddies[buddy]:
        order_buddies[buddy] = True
        order_free(addr & ~size, order + 1, False)
    else:
        order_buddies[oaddr] = False
        free_areas[order].push(addr)

# def free(addr: int, size: int, clear: bool = True):
#     global buddies, free_areas
# 
#     if clear:
#         memory[addr:addr+size] = [0] * size
#     i = 0
#     free_addr = addr + size
#     while 1 << i <= size:
#         if size & 1 << i:
#             free_addr -= 1 << i
#             order_free(free_addr, i, False)
#         i += 1


print("malloc:")
print_buddies()
a0 = order_malloc(3)
print_buddies()
a1 = order_malloc(2)
print_buddies()
a2 = order_malloc(1)
print_buddies()
a3 = order_malloc(1)
print_buddies()

print()
print("free:")
order_free(a0, 3)
print_buddies()
order_free(a1, 2)
print_buddies()
order_free(a2, 1)
print_buddies()
order_free(a3, 1)
print_buddies()

print()
print("malloc")
a0 = order_malloc(1)
print_buddies()

